//! Moving, copying, renaming and deleting photos and folders, keeping the catalogue in sync.
//! Moves between drives are done as copy -> verify -> delete, so nothing is lost if a
//! drive disconnects part-way through.

use crate::db;
use crate::indexer::{self, find_sidecar, Job};
use crate::state::AppState;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::params;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Emitter;

#[derive(Debug, Default, Serialize)]
pub struct OpReport {
    pub done: usize,
    pub failed: Vec<(String, String)>,
    /// True when the destination is not inside any library folder.
    pub outside_library: bool,
}

/// Pick a free name like "IMG_0001 (2).jpg" if the target exists.
pub fn unique_target(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for i in 2.. {
        let c = dir.join(format!("{stem} ({i}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

fn copy_verified(src: &Path, dst: &Path) -> Result<()> {
    let n = std::fs::copy(src, dst).with_context(|| format!("copying to {}", dst.display()))?;
    let expected = std::fs::metadata(src)?.len();
    if n != expected || std::fs::metadata(dst)?.len() != expected {
        let _ = std::fs::remove_file(dst);
        bail!("copy of {} was incomplete", src.display());
    }
    // Keep the original modification time.
    if let Ok(m) = std::fs::metadata(src).and_then(|m| m.modified()) {
        if let Ok(f) = std::fs::File::options().write(true).open(dst) {
            let _ = f.set_modified(m);
        }
    }
    Ok(())
}

/// Move a file, falling back to copy+delete across drives.
pub fn move_file(src: &Path, dst: &Path) -> Result<()> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_verified(src, dst)?;
    std::fs::remove_file(src).with_context(|| format!("removing {} after copying", src.display()))?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let t = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir_recursive(&e.path(), &t)?;
        } else {
            copy_verified(&e.path(), &t)?;
        }
    }
    Ok(())
}

pub(crate) fn sidecar_target(dst_photo: &Path, sidecar: &Path) -> PathBuf {
    // Keep the sidecar naming style: "name.xmp" or "name.ext.xmp".
    let side_name = sidecar.file_name().unwrap().to_string_lossy().to_string();
    let photo_name = dst_photo.file_name().unwrap().to_string_lossy().to_string();
    if side_name.to_lowercase().ends_with(&format!(".{}.xmp", indexer::ext_of(dst_photo))) {
        dst_photo.with_file_name(format!("{photo_name}.xmp"))
    } else {
        dst_photo.with_extension(sidecar.extension().unwrap_or_default())
    }
}

pub fn move_photos(state: &AppState, ids: &[i64], dest: &Path, copy: bool) -> Result<OpReport> {
    if !dest.is_dir() {
        bail!("The destination folder is not available: {}", dest.display());
    }
    let (photos, dest_root) = {
        let conn = state.conn.lock();
        (db::get_photos(&conn, ids)?, db::root_for_path(&conn, dest)?)
    };
    let mut rep = OpReport { outside_library: dest_root.is_none(), ..Default::default() };
    let total = photos.len();
    let mut new_files = vec![];
    let mut touched: Vec<PathBuf> = vec![dest.to_path_buf()];
    for (i, p) in photos.iter().enumerate() {
        let _ = state.app.emit(
            "op-progress",
            serde_json::json!({ "done": i, "total": total, "message": format!("{} {}", if copy { "Copying" } else { "Moving" }, p.filename) }),
        );
        let src = PathBuf::from(&p.path);
        if src.parent() == Some(dest) && !copy {
            continue;
        }
        let res: Result<PathBuf> = (|| {
            if !src.exists() {
                bail!("the file is not available (is its drive connected?)");
            }
            let dst = unique_target(dest, &p.filename);
            let side = find_sidecar(&src);
            if copy {
                copy_verified(&src, &dst)?;
                if let Some(s) = &side {
                    let _ = copy_verified(s, &sidecar_target(&dst, s));
                }
            } else {
                move_file(&src, &dst)?;
                if let Some(s) = &side {
                    let _ = move_file(s, &sidecar_target(&dst, s));
                }
            }
            Ok(dst)
        })();
        match res {
            Ok(dst) => {
                rep.done += 1;
                if let Some(par) = src.parent() {
                    touched.push(par.to_path_buf());
                }
                let conn = state.conn.lock();
                if copy {
                    new_files.push(dst);
                } else if let Some((root_id, _)) = &dest_root {
                    let filename = dst.file_name().unwrap().to_string_lossy().to_string();
                    conn.execute(
                        "UPDATE photos SET path = ?1, folder = ?2, filename = ?3, root_id = ?4 WHERE id = ?5",
                        params![dst.to_string_lossy(), dest.to_string_lossy(), filename, root_id, p.id],
                    )?;
                } else {
                    indexer::delete_ids(state, &conn, &[p.id])?;
                }
            }
            Err(e) => rep.failed.push((p.filename.clone(), e.to_string())),
        }
    }
    if !new_files.is_empty() && dest_root.is_some() {
        state.queue(Job::Files(new_files));
    }
    crate::notify::changed(&touched);
    let _ = state.app.emit("op-progress", serde_json::json!({ "done": total, "total": total, "message": "" }));
    Ok(rep)
}

pub fn move_folder(state: &AppState, src: &Path, dest_parent: &Path, copy: bool) -> Result<OpReport> {
    if !src.is_dir() {
        bail!("The folder is not available: {}", src.display());
    }
    if dest_parent.starts_with(src) {
        bail!("A folder cannot be moved into itself.");
    }
    let name = src.file_name().ok_or_else(|| anyhow!("invalid folder"))?.to_string_lossy().to_string();
    let dst = unique_target(dest_parent, &name);
    let dest_root = { db::root_for_path(&state.conn.lock(), dest_parent)? };
    let _ = state.app.emit("op-progress", serde_json::json!({ "done": 0, "total": 1, "message": format!("{} folder {}", if copy {"Copying"} else {"Moving"}, name) }));
    if copy {
        copy_dir_recursive(src, &dst)?;
    } else if std::fs::rename(src, &dst).is_err() {
        copy_dir_recursive(src, &dst)?;
        std::fs::remove_dir_all(src).context("removing the original folder after copying")?;
    }
    {
        let conn = state.conn.lock();
        let s = src.to_string_lossy().to_string();
        let d = dst.to_string_lossy().to_string();
        if !copy {
            match &dest_root {
                Some((rid, _)) => {
                    db::rewrite_prefix(&conn, &s, &d, Some(*rid))?;
                }
                None => {
                    let mut args = vec![];
                    let clause = db::folder_clause(&s, true, &mut args);
                    let mut stmt = conn.prepare(&format!("SELECT id FROM photos WHERE {clause}"))?;
                    let ids: Vec<i64> = stmt
                        .query_map(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    indexer::delete_ids(state, &conn, &ids)?;
                }
            }
        }
    }
    crate::notify::changed([src.parent().unwrap_or(src), dest_parent, dst.as_path()]);
    if dest_root.is_some() {
        state.queue(Job::ScanFolder(dst));
    }
    let _ = state.app.emit("op-progress", serde_json::json!({ "done": 1, "total": 1, "message": "" }));
    Ok(OpReport { done: 1, failed: vec![], outside_library: dest_root.is_none() })
}

pub(crate) fn check_name(name: &str) -> Result<()> {
    let n = name.trim();
    if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        bail!("\"{name}\" is not a valid name.");
    }
    Ok(())
}

pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf> {
    check_name(name)?;
    let p = parent.join(name.trim());
    if p.exists() {
        bail!("A folder named \"{}\" already exists here.", name.trim());
    }
    std::fs::create_dir(&p)?;
    crate::notify::changed([parent, p.as_path()]);
    Ok(p)
}

pub fn rename_folder(state: &AppState, path: &Path, new_name: &str) -> Result<PathBuf> {
    check_name(new_name)?;
    let dst = path.with_file_name(new_name.trim());
    if dst.exists() {
        bail!("A folder named \"{}\" already exists.", new_name.trim());
    }
    std::fs::rename(path, &dst)?;
    crate::notify::changed([dst.as_path()]);
    let conn = state.conn.lock();
    db::rewrite_prefix(&conn, &path.to_string_lossy(), &dst.to_string_lossy(), None)?;
    // If it was a library root, update that too.
    conn.execute(
        "UPDATE roots SET path = ?1, name = ?2 WHERE path = ?3",
        params![dst.to_string_lossy(), new_name.trim(), path.to_string_lossy()],
    )?;
    Ok(dst)
}

/// Move photos to the system Trash / Recycle Bin (or delete permanently).
pub fn delete_photos(state: &AppState, ids: &[i64], permanent: bool) -> Result<OpReport> {
    let photos = { db::get_photos(&state.conn.lock(), ids)? };
    let mut rep = OpReport::default();
    for p in photos {
        let src = PathBuf::from(&p.path);
        let mut targets = vec![src.clone()];
        if let Some(s) = find_sidecar(&src) {
            targets.push(s);
        }
        let r: Result<()> = if !src.exists() {
            Err(anyhow!("the file is not available (is its drive connected?)"))
        } else if permanent {
            targets.iter().try_for_each(|t| std::fs::remove_file(t).map_err(Into::into))
        } else {
            trash::delete_all(&targets).map_err(|e| anyhow!("could not move to Trash: {e}"))
        };
        match r {
            Ok(()) => {
                rep.done += 1;
                crate::notify::changed([src.as_path()]);
                indexer::delete_ids(state, &state.conn.lock(), &[p.id])?;
            }
            Err(e) => rep.failed.push((p.filename.clone(), e.to_string())),
        }
    }
    Ok(rep)
}

pub fn delete_folder(state: &AppState, path: &Path, permanent: bool) -> Result<()> {
    if !path.is_dir() {
        bail!("The folder is not available.");
    }
    if permanent {
        std::fs::remove_dir_all(path)?;
    } else {
        trash::delete(path).map_err(|e| anyhow!("could not move to Trash: {e}"))?;
    }
    if let Some(parent) = path.parent() {
        crate::notify::changed([parent]);
    }
    state.queue(Job::Removed(vec![path.to_path_buf()]));
    Ok(())
}

