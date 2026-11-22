//! Commands called from the user interface.

use crate::db::{self, Photo, PhotoLite, Query, Root};
use crate::fileops::{self, OpReport};
use crate::indexer::{self, Job};
use crate::metadata::{self, Edit};
use crate::openwith::{self, AppInfo};
use crate::state::AppState;
use crate::{ai, history, rename, share, watcher};
use rusqlite::params;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::sync::Arc;
use tauri::State;

type S<'a> = State<'a, Arc<AppState>>;
type R<T> = Result<T, String>;

fn e<E: std::fmt::Display>(x: E) -> String {
    x.to_string()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> R<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(e)?.map_err(e)
}

// ------------------------------------------------------------ library roots

#[tauri::command]
pub fn get_roots(state: S) -> R<Vec<Root>> {
    let roots = db::roots(&state.conn.lock()).map_err(e)?;
    Ok(db::with_online(roots))
}

#[tauri::command]
pub fn add_root(state: S, path: String) -> R<i64> {
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Err("That folder is not available.".into());
    }
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
    let id = {
        let conn = state.conn.lock();
        // A folder inside an existing library folder is already covered.
        let mut children = vec![];
        for r in db::roots(&conn).map_err(e)? {
            let rp = PathBuf::from(&r.path);
            if p.starts_with(&rp) {
                return Err(format!("This folder is already in your library (inside \"{}\").", r.name));
            }
            if rp.starts_with(&p) {
                children.push(r.id);
            }
        }
        // Adding a parent of existing library folders absorbs them: their photos (with
        // thumbnails, ratings and everything else) move under the new folder.
        let tx = conn.unchecked_transaction().map_err(e)?;
        tx.execute(
            "INSERT INTO roots (path, name, added_at) VALUES (?1, ?2, ?3)",
            params![path, name, crate::db::now_ms()],
        )
        .map_err(e)?;
        let id = tx.last_insert_rowid();
        for c in &children {
            tx.execute("UPDATE photos SET root_id = ?1 WHERE root_id = ?2", params![id, c]).map_err(e)?;
            tx.execute("DELETE FROM roots WHERE id = ?1", [c]).map_err(e)?;
        }
        tx.commit().map_err(e)?;
        id
    };
    state.queue(Job::ScanRoot(id));
    watcher::restart(&state);
    Ok(id)
}

#[tauri::command]
pub fn remove_root(state: S, id: i64) -> R<()> {
    {
        let conn = state.conn.lock();
        let ids: Vec<i64> = {
            let mut st = conn.prepare("SELECT id FROM photos WHERE root_id = ?1").map_err(e)?;
            let rows = st.query_map([id], |r| r.get(0)).map_err(e)?;
            rows.filter_map(|r| r.ok()).collect()
        };
        indexer::delete_ids(&state, &conn, &ids).map_err(e)?;
        conn.execute("DELETE FROM roots WHERE id = ?1", [id]).map_err(e)?;
    }
    watcher::restart(&state);
    Ok(())
}

#[tauri::command]
pub fn rescan(state: S, root_id: Option<i64>, folder: Option<String>) -> R<()> {
    if let Some(f) = folder {
        state.queue(Job::ScanFolder(PathBuf::from(f)));
    } else if let Some(id) = root_id {
        state.queue(Job::ScanRoot(id));
    } else {
        let roots = db::roots(&state.conn.lock()).map_err(e)?;
        for r in roots {
            state.queue(Job::ScanRoot(r.id));
        }
    }
    watcher::restart(&state);
    Ok(())
}

#[tauri::command]
pub fn refresh_photos(state: S, ids: Vec<i64>) -> R<()> {
    let photos = db::get_photos(&state.conn.lock(), &ids).map_err(e)?;
    state.queue(Job::Refresh(photos.into_iter().map(|p| PathBuf::from(p.path)).collect()));
    Ok(())
}

// ------------------------------------------------------------ folders

#[derive(Serialize)]
pub struct FolderNode {
    pub path: String,
    pub name: String,
    pub count: i64,
    pub has_children: bool,
    pub online: bool,
}

#[tauri::command]
pub fn folder_children(state: S, path: String) -> R<Vec<FolderNode>> {
    let base = path.trim_end_matches(['/', '\\']).to_string();
    let base = if base.is_empty() { path.clone() } else { base };
    // Only folders of the library are listed, never any other folder on the computer.
    in_library(&state, Path::new(&base), false).map_err(e)?;
    let prefix = format!("{base}{MAIN_SEPARATOR}");
    // Photo counts per immediate child, from the catalogue.
    let mut counts: HashMap<String, i64> = HashMap::new();
    let mut deeper: HashMap<String, bool> = HashMap::new();
    {
        let conn = state.conn.lock();
        let mut args = vec![];
        let clause = db::folder_clause(&base, true, &mut args);
        let mut st =
            conn.prepare(&format!("SELECT folder, COUNT(*) FROM photos WHERE {clause} GROUP BY folder")).map_err(e)?;
        let rows = st
            .query_map(rusqlite::params_from_iter(args.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(e)?;
        for (folder, n) in rows.flatten() {
            if let Some(rest) = folder.strip_prefix(&prefix) {
                let mut parts = rest.splitn(2, MAIN_SEPARATOR);
                let child = parts.next().unwrap_or("").to_string();
                if child.is_empty() {
                    continue;
                }
                *counts.entry(child.clone()).or_default() += n;
                if parts.next().is_some() {
                    deeper.insert(child, true);
                }
            }
        }
    }
    let base_path = Path::new(&base);
    let mut out = vec![];
    if base_path.is_dir() {
        if let Ok(rd) = std::fs::read_dir(base_path) {
            for ent in rd.flatten() {
                let p = ent.path();
                if !ent.file_type().map(|t| t.is_dir()).unwrap_or(false) || indexer::is_hidden(&p) {
                    continue;
                }
                let name = ent.file_name().to_string_lossy().to_string();
                let has_children = std::fs::read_dir(&p)
                    .map(|rd| {
                        rd.flatten().any(|c| {
                            c.file_type().map(|t| t.is_dir()).unwrap_or(false) && !indexer::is_hidden(&c.path())
                        })
                    })
                    .unwrap_or(false);
                out.push(FolderNode {
                    count: counts.get(&name).copied().unwrap_or(0),
                    path: p.to_string_lossy().to_string(),
                    name,
                    has_children,
                    online: true,
                });
            }
        }
    } else {
        for (name, count) in counts {
            out.push(FolderNode {
                path: format!("{prefix}{name}"),
                has_children: deeper.get(&name).copied().unwrap_or(false),
                name,
                count,
                online: false,
            });
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

/// Folder commands only touch folders inside the library. `strictly` also refuses a
/// library root itself (roots are removed from the library, never renamed, moved or
/// trashed from Lumio). Paths with ".." are refused so they can't step outside a root.
/// Links are followed: a link inside a library folder that points elsewhere is refused,
/// and with `strictly` (the folder itself is changed) a link is never acted on.
fn in_library(st: &AppState, path: &Path, strictly: bool) -> anyhow::Result<()> {
    plain_path(path)?;
    let root = db::root_for_path(&st.conn.lock(), path)?;
    match root {
        Some((_, root)) if !(strictly && root == path) => {
            fileops::inside_root(&root, path)?;
            if strictly {
                fileops::not_a_link(path)?;
            }
            Ok(())
        }
        Some(_) => anyhow::bail!("A library folder can't be changed here. Use Remove from library instead."),
        None => anyhow::bail!("\"{}\" is not a folder in your library.", path.display()),
    }
}

/// An absolute path without ".." (which could step out of wherever it seems to be).
fn plain_path(path: &Path) -> anyhow::Result<()> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        anyhow::bail!("\"{}\" is not a usable folder.", path.display());
    }
    Ok(())
}

/// A destination outside the library is fine when the person picked it (Move to…, Export
/// to…), but it must be a plain absolute folder, and one inside the library must really
/// be inside it.
fn usable_destination(st: &AppState, dest: &Path) -> anyhow::Result<()> {
    plain_path(dest)?;
    if let Some((_, root)) = db::root_for_path(&st.conn.lock(), dest)? {
        fileops::inside_root(&root, dest)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn create_folder(state: S<'_>, parent: String, name: String) -> R<String> {
    let st = state.inner().clone();
    blocking(move || {
        in_library(&st, Path::new(&parent), false)?;
        let p = fileops::create_folder(Path::new(&parent), &name)?;
        st.queue(Job::ScanFolder(p.clone()));
        let label = format!("Created folder {}", name.trim());
        history::record(&st.conn.lock(), &label, &history::Change::FolderCreate { path: p.clone() })?;
        Ok(p.to_string_lossy().to_string())
    })
    .await
}

#[tauri::command]
pub async fn rename_folder(state: S<'_>, path: String, name: String) -> R<String> {
    let st = state.inner().clone();
    let r = blocking(move || {
        let from = PathBuf::from(&path);
        in_library(&st, &from, true)?;
        let to = fileops::rename_folder(&*st, &from, &name)?;
        if to != from {
            let label =
                format!("Renamed folder {} to {}", from.file_name().unwrap_or_default().to_string_lossy(), name.trim());
            history::record(&st.conn.lock(), &label, &history::Change::FolderRename { from, to: to.clone() })?;
        }
        Ok(to.to_string_lossy().to_string())
    })
    .await;
    watcher::restart(&state);
    r
}

#[tauri::command]
pub async fn delete_folder(state: S<'_>, path: String, permanent: bool) -> R<()> {
    let st = state.inner().clone();
    blocking(move || {
        in_library(&st, Path::new(&path), true)?;
        fileops::delete_folder(&*st, Path::new(&path), permanent)?;
        let name = Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();
        let label = if permanent {
            format!("Deleted folder {name} permanently")
        } else {
            format!("Moved folder {name} to the Trash")
        };
        history::record(&st.conn.lock(), &label, &history::Change::Trash { count: 1, permanent })?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn move_folder(state: S<'_>, src: String, dest: String, copy: bool) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || {
        let from = PathBuf::from(&src);
        in_library(&st, &from, true)?;
        in_library(&st, Path::new(&dest), false)?;
        let mut rep = fileops::move_folder(&*st, &from, Path::new(&dest), copy)?;
        if let Some(to) = rep.folder.clone() {
            let name = from.file_name().unwrap_or_default().to_string_lossy().to_string();
            let into = Path::new(&dest).file_name().unwrap_or_default().to_string_lossy().to_string();
            let (label, change) = if copy {
                (format!("Copied folder {name} into {into}"), history::Change::FolderCopy { to })
            } else {
                (format!("Moved folder {name} into {into}"), history::Change::FolderMove { from, to })
            };
            rep.history = Some(history::record(&st.conn.lock(), &label, &change)?);
        }
        Ok(rep)
    })
    .await
}

// ------------------------------------------------------------ photos

#[tauri::command]
pub fn query_photos(state: S, q: Query) -> R<Vec<PhotoLite>> {
    db::query_photos(&state.conn.lock(), &q).map_err(e)
}

#[tauri::command]
pub fn get_photo(state: S, id: i64) -> R<Option<Photo>> {
    db::get_photo(&state.conn.lock(), id).map_err(e)
}

#[tauri::command]
pub fn get_photos(state: S, ids: Vec<i64>) -> R<Vec<Photo>> {
    db::get_photos(&state.conn.lock(), &ids).map_err(e)
}

#[tauri::command]
pub async fn move_photos(state: S<'_>, ids: Vec<i64>, dest: String, copy: bool) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || {
        usable_destination(&st, Path::new(&dest))?;
        let mut rep = fileops::move_photos(&*st, &ids, Path::new(&dest), copy)?;
        history::record_move(&st.conn.lock(), &mut rep, copy, Path::new(&dest))?;
        Ok(rep)
    })
    .await
}

/// The undo history, newest first.
#[tauri::command]
pub fn history_list(state: S) -> R<Vec<history::Entry>> {
    history::list(&state.conn.lock(), 300).map_err(e)
}

/// Undo a change and everything after it (or just the latest change, without an id).
#[tauri::command]
pub async fn history_undo(state: S<'_>, id: Option<i64>) -> R<history::UndoReport> {
    let st = state.inner().clone();
    let r = blocking(move || history::undo(&*st, id)).await;
    // Folders may have been renamed or moved back.
    watcher::restart(&state);
    r
}

/// Moves waiting for a drive to be connected.
#[tauri::command]
pub fn pending_moves(state: S) -> R<Vec<crate::pending::PendingMove>> {
    crate::pending::list(&state.conn.lock()).map_err(e)
}

/// Cancel waiting moves: the photos go back to where their files really are.
#[tauri::command]
pub fn cancel_pending_moves(state: S, ids: Vec<i64>) -> R<usize> {
    crate::pending::cancel(&state, &ids).map_err(e)
}

/// Try the waiting moves now (the worker also tries every minute).
#[tauri::command]
pub fn retry_pending_moves(state: S) -> R<()> {
    state.queue(Job::ApplyPending);
    Ok(())
}

/// Colour histogram of a photo, counted from the preview or thumbnail Lumio keeps for it.
#[tauri::command]
pub async fn histogram(state: S<'_>, id: i64) -> R<crate::histogram::Histogram> {
    let st = state.inner().clone();
    blocking(move || {
        let preview = crate::thumbs::offline_preview_path(&st.offline_dir, id);
        let thumb = crate::thumbs::thumb_path(&st.thumbs_dir, id);
        let src = if preview.exists() { preview } else { thumb };
        if !src.exists() {
            anyhow::bail!("No preview yet");
        }
        crate::histogram::from_file(&src)
    })
    .await
}

#[tauri::command]
pub async fn offline_preview_stats(state: S<'_>) -> R<crate::offline::Stats> {
    let st = state.inner().clone();
    blocking(move || crate::offline::stats(&st)).await
}

#[tauri::command]
pub async fn clear_offline_previews(state: S<'_>) -> R<()> {
    let st = state.inner().clone();
    blocking(move || {
        crate::offline::clear(&st)?;
        st.queue(Job::Previews);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn delete_photos(state: S<'_>, ids: Vec<i64>, permanent: bool) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || {
        let mut rep = fileops::delete_photos(&*st, &ids, permanent)?;
        history::record_trash(&st.conn.lock(), &mut rep, permanent)?;
        Ok(rep)
    })
    .await
}

#[tauri::command]
pub async fn rename_photo(state: S<'_>, id: i64, name: String) -> R<rename::RenameReport> {
    let st = state.inner().clone();
    blocking(move || {
        let mut rep = rename::rename_one(&st, id, &name)?;
        history::record_rename(&st.conn.lock(), &mut rep)?;
        Ok(rep)
    })
    .await
}

/// Show what a batch rename would do, without changing anything.
#[tauri::command]
pub async fn rename_preview(state: S<'_>, ids: Vec<i64>, opts: rename::BatchOptions) -> R<Vec<rename::PlannedName>> {
    let st = state.inner().clone();
    blocking(move || {
        let mut photos = db::get_photos(&st.conn.lock(), &ids)?;
        // Keep the order the photos are shown in.
        let pos: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        photos.sort_by_key(|p| pos.get(&p.id).copied().unwrap_or(usize::MAX));
        rename::plan(&photos, &opts)
    })
    .await
}

/// Rename photos to the given names (from a preview, or to undo an earlier rename).
#[tauri::command]
pub async fn rename_photos(state: S<'_>, items: Vec<(i64, String)>) -> R<rename::RenameReport> {
    let st = state.inner().clone();
    blocking(move || {
        let mut rep = rename::rename_many(&st, &items)?;
        history::record_rename(&st.conn.lock(), &mut rep)?;
        Ok(rep)
    })
    .await
}

/// Every keyword in the library with how many photos have it, most used first
/// (for suggestions while typing a keyword).
#[tauri::command]
pub async fn keyword_list(state: S<'_>) -> R<Vec<(String, i64)>> {
    let st = state.inner().clone();
    blocking(move || {
        let conn = st.conn.lock();
        let mut stmt = conn.prepare("SELECT keywords FROM photos WHERE keywords IS NOT NULL AND keywords != ''")?;
        let mut counts: HashMap<String, (String, i64)> = HashMap::new();
        for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
            for k in db::decode_keywords(Some(&row?)) {
                counts.entry(k.to_lowercase()).or_insert_with(|| (k.clone(), 0)).1 += 1;
            }
        }
        let mut out: Vec<(String, i64)> = counts.into_values().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        Ok(out)
    })
    .await
}

/// The "where do edits go" settings: RAW files (`sidecar` | `embed`) and every other
/// photo or video (`embed` | `sidecar`).
fn write_modes(st: &AppState) -> (String, String) {
    let conn = st.conn.lock();
    (
        db::get_setting(&conn, "raw_write_mode").unwrap_or_else(|| "sidecar".into()),
        db::get_setting(&conn, "image_write_mode").unwrap_or_else(|| "embed".into()),
    )
}

/// Do this photo's edits go into an XMP sidecar rather than the file itself?
pub(crate) fn edits_go_to_sidecar(ext: &str, raw_mode: &str, image_mode: &str) -> bool {
    if metadata::is_raw(ext) {
        raw_mode != "embed"
    } else {
        metadata::video_needs_sidecar(ext) || image_mode == "sidecar"
    }
}

/// Write metadata to the files (or their XMP sidecars) and update the catalogue.
#[tauri::command]
pub async fn update_metadata(state: S<'_>, ids: Vec<i64>, edit: Edit) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || {
        let photos = db::get_photos(&st.conn.lock(), &ids)?;
        let (raw_mode, image_mode) = write_modes(&st);
        let mut rep = OpReport::default();
        for p in photos {
            let path = PathBuf::from(&p.path);
            // Work out the keyword list the photo ends up with, and what was taken off.
            let keyword_change: Option<(Vec<String>, Vec<String>)> = if let Some(k) = &edit.keywords {
                let removed =
                    p.keywords.iter().filter(|x| !k.iter().any(|y| y.eq_ignore_ascii_case(x))).cloned().collect();
                Some((k.clone(), removed))
            } else if edit.add_keywords.is_some() || edit.remove_keywords.is_some() {
                let mut k = p.keywords.clone();
                for a in edit.add_keywords.iter().flatten() {
                    if !k.iter().any(|x| x.eq_ignore_ascii_case(a)) {
                        k.push(a.trim().to_string());
                    }
                }
                let rm = edit.remove_keywords.clone().unwrap_or_default();
                k.retain(|x| !rm.iter().any(|r| r.eq_ignore_ascii_case(x)));
                Some((k, rm))
            } else {
                None
            };
            let mut plan: Option<metadata::KeywordPlan> = None;
            let res: anyhow::Result<()> = (|| {
                if !path.exists() {
                    anyhow::bail!("the file is not available (is its drive connected?)");
                }
                let use_sidecar = edits_go_to_sidecar(&p.ext, &raw_mode, &image_mode);
                let target = if use_sidecar {
                    let side = indexer::sidecar_path(&path);
                    if !side.exists() {
                        st.et.create_sidecar(&path, &side)?;
                    }
                    side
                } else {
                    path.clone()
                };
                // Keyword paths (Lightroom's lr:hierarchicalSubject) already in the file.
                let kw = match &keyword_change {
                    Some((wanted, removed)) => {
                        let old_hier = st
                            .et
                            .read_json(std::slice::from_ref(&target))
                            .ok()
                            .and_then(|v| v.first().map(|x| metadata::parse(x).hier_keywords))
                            .unwrap_or_default();
                        let pl = metadata::plan_keywords(wanted, removed, &old_hier);
                        let had = !old_hier.is_empty();
                        plan = Some(pl);
                        Some((plan.as_ref().unwrap(), had))
                    }
                    None => None,
                };
                // Sidecars and MP4/MOV hold XMP but not EXIF or IPTC.
                let xmp_only = use_sidecar || metadata::is_video(&p.ext);
                st.et.write(&target, &metadata::write_args(&edit, kw, xmp_only))?;
                Ok(())
            })();
            match res {
                Ok(()) => {
                    rep.done += 1;
                    let conn = st.conn.lock();
                    // The same 0 to 5 range the file was given.
                    if let Some(r) = edit.rating {
                        conn.execute("UPDATE photos SET rating=?1 WHERE id=?2", params![r.clamp(0, 5), p.id])?;
                    }
                    if let Some(v) = &edit.label {
                        conn.execute("UPDATE photos SET label=?1 WHERE id=?2", params![nz(v), p.id])?;
                    }
                    if let Some(v) = &edit.title {
                        conn.execute("UPDATE photos SET title=?1 WHERE id=?2", params![nz(v), p.id])?;
                    }
                    if let Some(v) = &edit.caption {
                        conn.execute("UPDATE photos SET caption=?1 WHERE id=?2", params![nz(v), p.id])?;
                    }
                    if let Some(v) = &edit.creator {
                        conn.execute("UPDATE photos SET creator=?1 WHERE id=?2", params![nz(v), p.id])?;
                    }
                    if let Some(v) = &edit.copyright {
                        conn.execute("UPDATE photos SET copyright=?1 WHERE id=?2", params![nz(v), p.id])?;
                    }
                    if let Some(k) = &plan {
                        conn.execute(
                            "UPDATE photos SET keywords=?1 WHERE id=?2",
                            params![db::encode_keywords(&k.subject), p.id],
                        )?;
                    }
                    if let Some(g) = &edit.gps_change() {
                        let (la, lo) = g.map(|(a, b)| (Some(a), Some(b))).unwrap_or((None, None));
                        conn.execute("UPDATE photos SET lat=?1, lon=?2 WHERE id=?3", params![la, lo, p.id])?;
                    }
                    // The newer of the photo and its sidecar, as the indexer stores it, so
                    // the next scan sees nothing new.
                    if let Some((size, mtime)) = indexer::stat_with_sidecar(&path) {
                        conn.execute("UPDATE photos SET size=?1, mtime=?2 WHERE id=?3", params![size, mtime, p.id])?;
                    }
                }
                Err(err) => rep.failed.push((p.filename.clone(), err.to_string())),
            }
        }
        Ok(rep)
    })
    .await
}

/// Turn or flip photos without re-encoding them: only the orientation tag changes.
/// RAW files keep it in their XMP sidecar (unless RAW files are set to be written directly).
#[tauri::command]
pub async fn rotate_photos(state: S<'_>, ids: Vec<i64>, op: String) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || {
        let photos = db::get_photos(&st.conn.lock(), &ids)?;
        let (raw_mode, image_mode) = write_modes(&st);
        let mut rep = OpReport::default();
        for p in photos {
            let path = PathBuf::from(&p.path);
            let Some(new_o) = metadata::transform_orientation(p.orientation, &op) else {
                anyhow::bail!("unknown rotation {op}");
            };
            let res: anyhow::Result<()> = (|| {
                if !path.exists() {
                    anyhow::bail!("the file is not available (is its drive connected?)");
                }
                if metadata::is_video(&p.ext) {
                    anyhow::bail!("videos can't be rotated in Lumio");
                }
                let use_sidecar = edits_go_to_sidecar(&p.ext, &raw_mode, &image_mode);
                if matches!(p.ext.as_str(), "gif" | "bmp") && !use_sidecar {
                    anyhow::bail!("{} files can't store a rotation", p.ext.to_uppercase());
                }
                if use_sidecar {
                    let side = indexer::sidecar_path(&path);
                    if !side.exists() {
                        st.et.create_sidecar(&path, &side)?;
                    }
                    st.et.write(&side, &[format!("-XMP-tiff:Orientation#={new_o}")])?;
                } else {
                    st.et.write(&path, &[format!("-Orientation#={new_o}")])?;
                }
                Ok(())
            })();
            match res {
                Ok(()) => {
                    rep.done += 1;
                    let (size, mtime) = indexer::stat_with_sidecar(&path).unwrap_or((p.size, p.mtime));
                    let swap = metadata::is_quarter_turn(p.orientation) != metadata::is_quarter_turn(Some(new_o));
                    {
                        let conn = st.conn.lock();
                        conn.execute(
                            "UPDATE photos SET orientation = ?2, size = ?3, mtime = ?4,
                                width = CASE WHEN ?5 THEN height ELSE width END,
                                height = CASE WHEN ?5 THEN width ELSE height END WHERE id = ?1",
                            params![p.id, new_o, size, mtime, swap],
                        )?;
                    }
                    let dest = crate::thumbs::thumb_path(&st.thumbs_dir, p.id);
                    // Turn the offline preview too, if the photo has one.
                    let size = (p.preview > 0).then_some(p.preview as u32);
                    let pdest = crate::thumbs::offline_preview_path(&st.offline_dir, p.id);
                    let preview = size.map(|s| (pdest.as_path(), s));
                    match crate::thumbs::make_thumb(&path, &p.ext, Some(new_o), &dest, preview, &st.et, &st.tmp_dir) {
                        Ok(made) => {
                            let pv = match made.preview {
                                Some(false) => 0,
                                _ => p.preview,
                            };
                            st.conn.lock().execute(
                                "UPDATE photos SET thumb = 1, preview = ?2 WHERE id = ?1",
                                params![p.id, pv],
                            )?;
                        }
                        Err(err) => log::warn!("thumbnail after rotate failed for {}: {err:#}", path.display()),
                    }
                }
                Err(err) => {
                    log::warn!("rotate failed for {}: {err:#}", path.display());
                    rep.failed.push((p.filename.clone(), err.to_string()))
                }
            }
        }
        Ok(rep)
    })
    .await
}

/// Store a video thumbnail captured by the app's player (a JPEG as base64), for videos
/// the system could not make a thumbnail for. Also fills in the size and length if the
/// file's metadata did not have them.
#[tauri::command]
pub async fn save_video_thumb(
    state: S<'_>,
    id: i64,
    jpeg: String,
    width: Option<i64>,
    height: Option<i64>,
    duration: Option<f64>,
) -> R<()> {
    use base64::Engine;
    let st = state.inner().clone();
    blocking(move || {
        let Some(p) = db::get_photo(&st.conn.lock(), id)? else { anyhow::bail!("not in the catalogue") };
        if !metadata::is_video(&p.ext) {
            anyhow::bail!("not a video");
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(jpeg.trim())?;
        crate::thumbs::save_frame(&bytes, &crate::thumbs::thumb_path(&st.thumbs_dir, id))?;
        let duration = duration.filter(|d| d.is_finite() && *d > 0.0);
        st.conn.lock().execute(
            "UPDATE photos SET thumb = 1, thumb_tries = 0, width = coalesce(width, ?2), height = coalesce(height, ?3), duration = coalesce(duration, ?4) WHERE id = ?1",
            params![id, width.filter(|w| *w > 0), height.filter(|h| *h > 0), duration],
        )?;
        Ok(())
    })
    .await
}

// ------------------------------------------------------------ stacks (groups of shots)

/// Group photos into one stack. Stacks the photos already belong to are merged in.
/// `top` is the cover photo shown when the stack is closed.
#[tauri::command]
pub fn stack_photos(state: S, ids: Vec<i64>, top: Option<i64>) -> R<i64> {
    if ids.len() < 2 {
        return Err("Select at least two photos to group".into());
    }
    let conn = state.conn.lock();
    let tx = conn.unchecked_transaction().map_err(e)?;
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let sid: i64 = tx.query_row("SELECT coalesce(MAX(stack_id), 0) + 1 FROM photos", [], |r| r.get(0)).map_err(e)?;
    let top = top.filter(|t| ids.contains(t)).unwrap_or(ids[0]);
    tx.execute(
        &format!(
            "UPDATE photos SET stack_id = ?1, stack_top = (id = ?2) WHERE id IN ({list})
             OR stack_id IN (SELECT stack_id FROM photos WHERE id IN ({list}) AND stack_id IS NOT NULL)"
        ),
        params![sid, top],
    )
    .map_err(e)?;
    tx.commit().map_err(e)?;
    Ok(sid)
}

/// Take photos out of their stacks. With `whole`, every stack they belong to is dissolved.
#[tauri::command]
pub fn unstack_photos(state: S, ids: Vec<i64>, whole: bool) -> R<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let conn = state.conn.lock();
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let sql = if whole {
        format!("UPDATE photos SET stack_id = NULL, stack_top = 0 WHERE stack_id IN (SELECT stack_id FROM photos WHERE id IN ({list}) AND stack_id IS NOT NULL)")
    } else {
        format!("UPDATE photos SET stack_id = NULL, stack_top = 0 WHERE id IN ({list})")
    };
    conn.execute(&sql, []).map_err(e)?;
    db::tidy_stacks(&conn).map_err(e)
}

/// Make this photo the cover of its stack.
#[tauri::command]
pub fn set_stack_top(state: S, id: i64) -> R<()> {
    let conn = state.conn.lock();
    conn.execute(
        "UPDATE photos SET stack_top = (id = ?1) WHERE stack_id = (SELECT stack_id FROM photos WHERE id = ?1)",
        [id],
    )
    .map_err(e)?;
    Ok(())
}

fn nz(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

// ------------------------------------------------------------ open with

#[tauri::command]
pub fn list_open_with(state: S, id: i64) -> R<Vec<AppInfo>> {
    let p = db::get_photo(&state.conn.lock(), id).map_err(e)?.ok_or("photo not found")?;
    Ok(openwith::list(Path::new(&p.path)))
}

#[tauri::command]
pub fn open_with(state: S, ids: Vec<i64>, app: String) -> R<()> {
    let photos = db::get_photos(&state.conn.lock(), &ids).map_err(e)?;
    for p in photos {
        openwith::open_with(Path::new(&p.path), &app).map_err(e)?;
    }
    Ok(())
}

/// "Other application…": shows the system's file dialog from here, so only an application
/// the person picks in that dialog can be started (never a path sent by the interface).
#[tauri::command]
pub async fn choose_app_and_open(state: S<'_>, ids: Vec<i64>) -> R<bool> {
    use tauri_plugin_dialog::DialogExt;
    let photos = db::get_photos(&state.conn.lock(), &ids).map_err(e)?;
    let app = state.app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut d = app.dialog().file().set_title("Choose an application");
        if cfg!(target_os = "macos") {
            d = d.add_filter("Applications", &["app"]).set_directory("/Applications");
        } else if cfg!(target_os = "windows") {
            d = d.add_filter("Programs", &["exe"]);
        }
        d.blocking_pick_file()
    })
    .await
    .map_err(e)?;
    let Some(chosen) = picked.and_then(|f| f.into_path().ok()) else { return Ok(false) };
    for p in photos {
        openwith::open_with_chosen(Path::new(&p.path), &chosen).map_err(e)?;
    }
    Ok(true)
}

// ------------------------------------------------------------ settings

#[tauri::command]
pub fn get_settings(state: S) -> R<HashMap<String, String>> {
    let conn = state.conn.lock();
    let mut st = conn.prepare("SELECT key, value FROM settings WHERE key NOT LIKE 'key_%'").map_err(e)?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn set_setting(state: S, key: String, value: String) -> R<()> {
    if key.starts_with("key_") {
        return Err("use ai_set_key".into());
    }
    if key == "ai_openai_base_url" {
        ai::check_base_url(&value).map_err(e)?;
    }
    db::set_setting(&state.conn.lock(), &key, &value).map_err(e)?;
    if key.starts_with("offline_preview") {
        state.queue(Job::Previews);
    }
    Ok(())
}

#[derive(Serialize)]
pub struct AppStatus {
    pub version: String,
    pub exiftool: bool,
    pub exiftool_path: String,
    pub data_dir: String,
    pub photo_count: i64,
}

#[tauri::command]
pub fn app_status(state: S) -> R<AppStatus> {
    let n: i64 = state.conn.lock().query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0)).map_err(e)?;
    Ok(AppStatus {
        version: env!("CARGO_PKG_VERSION").into(),
        exiftool: state.et.available(),
        exiftool_path: state.et.describe(),
        data_dir: state.db_path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        photo_count: n,
    })
}

// ------------------------------------------------------------ AI

#[tauri::command]
pub fn ai_settings(state: S) -> ai::AiSettings {
    ai::settings(&state)
}

#[tauri::command]
pub fn ai_set_key(which: String, key: String) -> R<()> {
    ai::set_key(&which, &key).map_err(e)
}

#[tauri::command]
pub async fn ai_chat(state: S<'_>, text: String) -> R<ai::ChatReply> {
    ai::chat(state.inner().clone(), text).await.map_err(e)
}

#[tauri::command]
pub fn ai_reset(state: S) {
    ai::reset(&state)
}

// ------------------------------------------------------------ system integration

#[tauri::command]
pub fn reveal(state: S, id: Option<i64>, path: Option<String>) -> R<()> {
    let p = match (id, path) {
        (Some(id), _) => db::get_photo(&state.conn.lock(), id).map_err(e)?.ok_or("photo not found")?.path,
        (None, Some(p)) => {
            // A folder from the folder list: only ones in the library.
            in_library(&state, Path::new(&p), false).map_err(e)?;
            p
        }
        _ => return Err("nothing to show".into()),
    };
    if !Path::new(&p).exists() {
        return Err("This item is not available (is its drive connected?)".into());
    }
    tauri_plugin_opener::reveal_item_in_dir(&p).map_err(e)
}

#[tauri::command]
pub fn open_default(state: S, ids: Vec<i64>) -> R<()> {
    for p in db::get_photos(&state.conn.lock(), &ids).map_err(e)? {
        if !Path::new(&p.path).exists() {
            return Err(format!("{} is not available (is its drive connected?)", p.filename));
        }
        tauri_plugin_opener::open_path(&p.path, None::<&str>).map_err(e)?;
    }
    Ok(())
}

// ------------------------------------------------------------ sharing

fn shared(state: &S) -> Arc<AppState> {
    state.inner().clone()
}

/// Which sharing options this computer offers.
#[tauri::command]
pub fn share_options() -> serde_json::Value {
    serde_json::json!({
        "sheet": share::has_sheet(),
        "sites": share::SITES.iter().map(|(k, n, _)| serde_json::json!({ "key": k, "name": n })).collect::<Vec<_>>(),
    })
}

#[tauri::command]
pub async fn share_sheet(state: S<'_>, ids: Vec<i64>, x: f64, y: f64) -> R<share::Outcome> {
    let st = shared(&state);
    blocking(move || share::sheet(&st, &ids, x, y)).await
}

#[tauri::command]
pub async fn share_email(state: S<'_>, ids: Vec<i64>) -> R<share::Outcome> {
    let st = shared(&state);
    blocking(move || share::email(&st, &ids)).await
}

#[tauri::command]
pub async fn share_whatsapp(state: S<'_>, ids: Vec<i64>, web: bool) -> R<share::Outcome> {
    let st = shared(&state);
    blocking(move || share::whatsapp(&st, &ids, web)).await
}

#[tauri::command]
pub async fn share_site(state: S<'_>, ids: Vec<i64>, site: String) -> R<share::Outcome> {
    let st = shared(&state);
    blocking(move || share::site(&st, &ids, &site)).await
}

#[tauri::command]
pub async fn share_copy(state: S<'_>, ids: Vec<i64>) -> R<share::Outcome> {
    let st = shared(&state);
    blocking(move || share::copy_only(&st, &ids)).await
}

#[tauri::command]
pub async fn share_show(state: S<'_>, ids: Vec<i64>) -> R<share::Outcome> {
    let st = shared(&state);
    blocking(move || share::show_copies(&st, &ids)).await
}
