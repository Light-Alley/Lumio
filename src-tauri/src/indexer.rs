//! Background indexing: scans folders, reads metadata and builds thumbnails.
//! All work happens on one worker thread fed by a channel, so the UI stays responsive.

use crate::db;
use crate::metadata::{self, Meta};
use crate::state::AppState;
use crate::thumbs;
use anyhow::Result;
use rayon::prelude::*;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tauri::Emitter;

#[derive(Debug, Clone)]
pub enum Job {
    /// Scan a whole library root.
    ScanRoot(i64),
    /// Scan one folder (recursively), e.g. after a folder was created or copied in.
    ScanFolder(PathBuf),
    /// (Re)index specific files if they changed.
    Files(Vec<PathBuf>),
    /// Re-read metadata and thumbnails even if the files look unchanged.
    Refresh(Vec<PathBuf>),
    /// Paths that disappeared.
    Removed(Vec<PathBuf>),
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub active: bool,
    pub done: usize,
    pub total: usize,
    pub message: String,
}

pub fn is_hidden(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            n.starts_with('.')
                || n.starts_with("._")
                || matches!(n, "$RECYCLE.BIN" | "System Volume Information" | "@eaDir" | "#recycle" | "#snapshot" | "node_modules")
        })
        .unwrap_or(false)
}

pub fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

pub fn find_sidecar(p: &Path) -> Option<PathBuf> {
    let a = p.with_extension("xmp");
    if a.exists() {
        return Some(a);
    }
    let b = p.with_extension("XMP");
    if b.exists() {
        return Some(b);
    }
    let mut c = p.as_os_str().to_owned();
    c.push(".xmp");
    let c = PathBuf::from(c);
    if c.exists() {
        return Some(c);
    }
    None
}

fn stat(p: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    if !m.is_file() {
        return None;
    }
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some((m.len() as i64, mtime))
}

pub fn run_worker(state: Arc<AppState>, rx: Receiver<Job>) {
    let conn = match db::open(&state.db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("indexer cannot open db: {e}");
            return;
        }
    };
    while let Ok(first) = rx.recv() {
        // Coalesce whatever else is queued.
        let mut jobs = vec![first];
        std::thread::sleep(Duration::from_millis(150));
        while let Ok(j) = rx.try_recv() {
            jobs.push(j);
        }
        let mut files: Vec<PathBuf> = vec![];
        let mut refresh: Vec<PathBuf> = vec![];
        let mut removed: Vec<PathBuf> = vec![];
        let mut scans: Vec<Job> = vec![];
        for j in jobs {
            match j {
                Job::Files(f) => files.extend(f),
                Job::Refresh(f) => refresh.extend(f),
                Job::Removed(f) => removed.extend(f),
                other => scans.push(other),
            }
        }
        if !removed.is_empty() {
            if let Err(e) = handle_removed(&state, &conn, &removed) {
                log::warn!("remove: {e}");
            }
        }
        for s in scans {
            let r = match s {
                Job::ScanRoot(id) => scan_root(&state, &conn, id),
                Job::ScanFolder(p) => scan_folder(&state, &conn, &p),
                _ => Ok(()),
            };
            if let Err(e) = r {
                log::warn!("scan failed: {e}");
            }
        }
        if !files.is_empty() {
            let _ = index_paths(&state, &conn, dedup(files), false);
        }
        if !refresh.is_empty() {
            let _ = index_paths(&state, &conn, dedup(refresh), true);
        }
        emit_progress(&state, false, 0, 0, "");
        let _ = state.app.emit("library-changed", ());
    }
}

fn dedup(v: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    v.into_iter().filter(|p| seen.insert(p.clone())).collect()
}

fn emit_progress(state: &AppState, active: bool, done: usize, total: usize, msg: &str) {
    let _ = state.app.emit(
        "index-progress",
        Progress { active, done, total, message: msg.to_string() },
    );
}

fn handle_removed(state: &AppState, conn: &Connection, paths: &[PathBuf]) -> Result<()> {
    for p in paths {
        if p.exists() {
            continue;
        }
        let ps = p.to_string_lossy().to_string();
        let mut args = vec![];
        let clause = db::folder_clause(&ps, true, &mut args);
        args.push(ps.clone().into());
        let sql = format!("SELECT id FROM photos WHERE path = ?{} OR {clause}", args.len());
        let ids: Vec<i64> = {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        delete_ids(state, conn, &ids)?;
    }
    Ok(())
}

pub fn delete_ids(state: &AppState, conn: &Connection, ids: &[i64]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for id in ids {
        tx.execute("DELETE FROM photos WHERE id = ?1", [id])?;
        let _ = std::fs::remove_file(thumbs::thumb_path(&state.thumbs_dir, *id));
    }
    tx.commit()?;
    Ok(())
}

fn scan_root(state: &AppState, conn: &Connection, root_id: i64) -> Result<()> {
    let path: String = conn.query_row("SELECT path FROM roots WHERE id = ?1", [root_id], |r| r.get(0))?;
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Ok(()); // offline: keep catalogue as is
    }
    scan_dir(state, conn, &p, root_id)?;
    conn.execute(
        "UPDATE roots SET last_scan = ?1 WHERE id = ?2",
        params![chrono::Utc::now().timestamp_millis(), root_id],
    )?;
    Ok(())
}

fn scan_folder(state: &AppState, conn: &Connection, p: &Path) -> Result<()> {
    if !p.is_dir() {
        return Ok(());
    }
    if let Some((root_id, _)) = db::root_for_path(conn, p)? {
        scan_dir(state, conn, p, root_id)?;
    }
    Ok(())
}

fn scan_dir(state: &AppState, conn: &Connection, dir: &Path, _root_id: i64) -> Result<()> {
    emit_progress(state, true, 0, 0, &format!("Scanning {}", dir.display()));
    let mut found: Vec<PathBuf> = vec![];
    let walker = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !is_hidden(e.path()));
    for e in walker.flatten() {
        if e.file_type().is_file() && metadata::is_supported(&ext_of(e.path())) {
            found.push(e.into_path());
        }
    }
    // Forget files that no longer exist below this folder.
    let present: HashSet<String> = found.iter().map(|p| p.to_string_lossy().to_string()).collect();
    let ds = dir.to_string_lossy().to_string();
    let mut args = vec![];
    let clause = db::folder_clause(&ds, true, &mut args);
    let gone: Vec<i64> = {
        let mut stmt = conn.prepare(&format!("SELECT id, path FROM photos WHERE {clause}"))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.filter_map(|r| r.ok()).filter(|(_, p)| !present.contains(p)).map(|(id, _)| id).collect()
    };
    if !gone.is_empty() {
        delete_ids(state, conn, &gone)?;
    }
    index_paths(state, conn, found, false)
}

struct Pending {
    path: PathBuf,
    ext: String,
    root_id: i64,
    size: i64,
    mtime: i64,
    existing: Option<i64>,
}

/// Index the given files (skipping ones whose size and mtime are unchanged unless `force`).
pub fn index_paths(state: &AppState, conn: &Connection, paths: Vec<PathBuf>, force: bool) -> Result<()> {
    let roots: Vec<(i64, PathBuf)> = db::list_roots(conn)?
        .into_iter()
        .map(|r| (r.id, PathBuf::from(r.path)))
        .collect();
    let mut todo: Vec<Pending> = vec![];
    for p in paths {
        let ext = ext_of(&p);
        if !metadata::is_supported(&ext) || is_hidden(&p) {
            continue;
        }
        let Some(root_id) = roots
            .iter()
            .filter(|(_, rp)| p.starts_with(rp))
            .max_by_key(|(_, rp)| rp.as_os_str().len())
            .map(|(id, _)| *id)
        else {
            continue;
        };
        let Some((size, mtime)) = stat(&p) else { continue };
        let ps = p.to_string_lossy().to_string();
        let existing = db::photo_by_path(conn, &ps)?;
        if let Some((_, s, m)) = existing {
            if !force && s == size && m == mtime {
                // Also refresh when the sidecar is newer than our last index.
                let side_newer = find_sidecar(&p)
                    .and_then(|sc| stat(&sc))
                    .map(|(_, sm)| sm > m)
                    .unwrap_or(false);
                let has_thumb: bool = conn
                    .query_row("SELECT thumb FROM photos WHERE path = ?1", [&ps], |r| r.get::<_, i64>(0))
                    .map(|t| t != 0)
                    .unwrap_or(true);
                if !side_newer && has_thumb {
                    continue;
                }
            }
        }
        todo.push(Pending { path: p, ext, root_id, size, mtime, existing: existing.map(|e| e.0) });
    }
    let total = todo.len();
    if total == 0 {
        return Ok(());
    }
    let mut done = 0;
    for chunk in todo.chunks(24) {
        emit_progress(state, true, done, total, &format!("Indexing {} of {}", done + 1, total));
        // 1. Metadata (image files plus any XMP sidecars), in one ExifTool call.
        let mut files: Vec<PathBuf> = chunk.iter().map(|c| c.path.clone()).collect();
        let sidecars: Vec<Option<PathBuf>> = chunk.iter().map(|c| find_sidecar(&c.path)).collect();
        files.extend(sidecars.iter().flatten().cloned());
        let mut by_path: HashMap<String, Meta> = HashMap::new();
        if state.et.available() {
            match state.et.read_json(&files) {
                Ok(vals) => {
                    for v in vals {
                        if let Some(src) = v.get("SourceFile").and_then(|s| s.as_str()) {
                            by_path.insert(normalize_src(src), metadata::parse(&v));
                        }
                    }
                }
                Err(e) => log::warn!("exiftool read failed: {e}"),
            }
        }
        // 2. Upsert rows.
        let now = chrono::Utc::now().timestamp_millis();
        let mut ids: Vec<(i64, PathBuf, String, Option<i64>)> = vec![];
        {
            let tx = conn.unchecked_transaction()?;
            for (c, sc) in chunk.iter().zip(sidecars.iter()) {
                let ps = c.path.to_string_lossy().to_string();
                let mut m = by_path.get(&normalize_src(&ps)).cloned().unwrap_or_default();
                if let Some(sc) = sc {
                    if let Some(side) = by_path.get(&normalize_src(&sc.to_string_lossy())) {
                        metadata::merge_sidecar(&mut m, side);
                    }
                }
                let folder = c.path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                let filename = c.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let kw = db::encode_keywords(&m.keywords);
                let id = if let Some(id) = c.existing {
                    tx.execute(
                        "UPDATE photos SET root_id=?2, folder=?3, filename=?4, ext=?5, size=?6, mtime=?7, width=?8, height=?9, orientation=?10, taken_at=?11, rating=?12, label=?13, title=?14, caption=?15, keywords=?16, creator=?17, copyright=?18, lat=?19, lon=?20, alt=?21, place=?22, make=?23, model=?24, lens=?25, iso=?26, fnumber=?27, exposure=?28, focal=?29 WHERE id=?1",
                        params![id, c.root_id, folder, filename, c.ext, c.size, c.mtime, m.width, m.height, m.orientation, m.taken_at, m.rating, m.label, m.title, m.caption, kw, m.creator, m.copyright, m.lat, m.lon, m.alt, m.place, m.make, m.model, m.lens, m.iso, m.fnumber, m.exposure, m.focal],
                    )?;
                    id
                } else {
                    tx.execute(
                        "INSERT INTO photos (root_id, path, folder, filename, ext, size, mtime, width, height, orientation, taken_at, rating, label, title, caption, keywords, creator, copyright, lat, lon, alt, place, make, model, lens, iso, fnumber, exposure, focal, indexed_at)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30)",
                        params![c.root_id, ps, folder, filename, c.ext, c.size, c.mtime, m.width, m.height, m.orientation, m.taken_at, m.rating, m.label, m.title, m.caption, kw, m.creator, m.copyright, m.lat, m.lon, m.alt, m.place, m.make, m.model, m.lens, m.iso, m.fnumber, m.exposure, m.focal, now],
                    )?;
                    tx.last_insert_rowid()
                };
                ids.push((id, c.path.clone(), c.ext.clone(), m.orientation));
            }
            tx.commit()?;
        }
        // 3. Thumbnails, in parallel.
        let results: Vec<(i64, Option<(u32, u32)>)> = ids
            .par_iter()
            .map(|(id, path, ext, orient)| {
                let dest = thumbs::thumb_path(&state.thumbs_dir, *id);
                let r = thumbs::make_thumb(path, ext, *orient, &dest, &state.et, &state.tmp_dir);
                if let Err(e) = &r {
                    log::warn!("thumbnail failed for {}: {e}", path.display());
                }
                (*id, r.ok())
            })
            .collect();
        {
            let tx = conn.unchecked_transaction()?;
            for (id, wh) in results {
                match wh {
                    Some((w, h)) => tx.execute(
                        "UPDATE photos SET thumb = 1, width = coalesce(width, ?2), height = coalesce(height, ?3) WHERE id = ?1",
                        params![id, w, h],
                    )?,
                    None => tx.execute("UPDATE photos SET thumb = 0 WHERE id = ?1", [id])?,
                };
            }
            tx.commit()?;
        }
        done += chunk.len();
        let _ = state.app.emit("library-changed", ());
    }
    emit_progress(state, true, total, total, "Done");
    Ok(())
}

/// ExifTool reports paths with forward slashes on Windows.
pub fn normalize_src(s: &str) -> String {
    if cfg!(windows) {
        s.replace('/', "\\").to_lowercase()
    } else {
        s.to_string()
    }
}
