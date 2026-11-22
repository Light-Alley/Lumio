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
    /// Do moves that were waiting for a drive, if it's back.
    ApplyPending,
    /// Make some of the offline previews that are still missing.
    Previews,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub active: bool,
    pub done: usize,
    pub total: usize,
    pub message: String,
}

/// Files that fail this many times in a row are left alone until they change
/// (or the user asks to rebuild their thumbnails).
const MAX_THUMB_TRIES: i64 = 3;

/// A library drive stopped answering part-way through. Indexing pauses and tries again later.
#[derive(Debug)]
pub struct Offline {
    pub root_id: i64,
    pub root: PathBuf,
}

impl std::fmt::Display for Offline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not reachable", self.root.display())
    }
}

impl std::error::Error for Offline {}

/// Can the folder still be listed? Network drives and sync clients (pCloud, Dropbox, SMB)
/// stop answering when the computer sleeps or the connection drops.
pub fn reachable(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut rd| {
            let _ = rd.next();
            true
        })
        .unwrap_or(false)
}

/// Can we actually read the start of the file?
pub fn readable(p: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 4096];
    std::fs::File::open(p).and_then(|mut f| f.read(&mut buf)).map(|n| n > 0).unwrap_or(false)
}

/// Stop the computer from idle-sleeping while indexing (macOS). Closing the lid still sleeps.
struct KeepAwake(Option<std::process::Child>);

impl KeepAwake {
    fn new() -> Self {
        #[cfg(target_os = "macos")]
        {
            let child = std::process::Command::new("/usr/bin/caffeinate")
                .args(["-i", "-w", &std::process::id().to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            KeepAwake(child)
        }
        #[cfg(not(target_os = "macos"))]
        KeepAwake(None)
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub fn is_hidden(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            n.starts_with('.')
                || n.starts_with("._")
                || matches!(
                    n,
                    "$RECYCLE.BIN" | "System Volume Information" | "@eaDir" | "#recycle" | "#snapshot" | "node_modules"
                )
        })
        .unwrap_or(false)
}

pub fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// The XMP sidecar that belongs to `p`, if there is one.
///
/// "IMG_1.CR2.xmp" always belongs to that file. "IMG_1.xmp" is the RAW file's sidecar
/// (Lightroom, Bridge and Capture One name them this way), so a JPEG shot alongside the
/// RAW (IMG_1.JPG next to IMG_1.CR2) does not take it: otherwise the JPEG would show,
/// and a move would carry off, the RAW file's rating and keywords.
pub fn find_sidecar(p: &Path) -> Option<PathBuf> {
    let mut c = p.as_os_str().to_owned();
    c.push(".xmp");
    let full = PathBuf::from(c);
    let owns_stem = metadata::is_raw(&ext_of(p)) || metadata::video_needs_sidecar(&ext_of(p));
    if !owns_stem && full.exists() {
        return Some(full);
    }
    for stem in [p.with_extension("xmp"), p.with_extension("XMP")] {
        if stem.exists() && (owns_stem || !has_sibling(p, true)) {
            return Some(stem);
        }
    }
    if full.exists() {
        return Some(full);
    }
    None
}

/// Where a new sidecar for `p` goes: "IMG_1.xmp", or "IMG_1.JPG.xmp" when another file
/// shares the name (a RAW twin, or the video half of an iPhone Live Photo).
pub fn sidecar_path(p: &Path) -> PathBuf {
    if let Some(s) = find_sidecar(p) {
        return s;
    }
    let owns_stem = metadata::is_raw(&ext_of(p)) || metadata::video_needs_sidecar(&ext_of(p));
    if !owns_stem && (has_sibling(p, false) || p.with_extension("xmp").exists()) {
        let mut c = p.as_os_str().to_owned();
        c.push(".xmp");
        return PathBuf::from(c);
    }
    p.with_extension("xmp")
}

/// Is there another photo or video with the same name next to `p`? With `owners_only`,
/// count only RAW files (and videos that keep a sidecar), which own "name.xmp".
fn has_sibling(p: &Path, owners_only: bool) -> bool {
    let me = ext_of(p);
    let exts = metadata::RAW_EXTS.iter().chain(metadata::VIDEO_EXTS.iter()).chain(metadata::IMAGE_EXTS.iter());
    exts.filter(|e| **e != me).any(|e| {
        (!owners_only || metadata::is_raw(e) || metadata::video_needs_sidecar(e))
            && (p.with_extension(e).exists() || p.with_extension(e.to_ascii_uppercase()).exists())
    })
}

/// The names in folders, read once per folder, so finding sidecars and RAW/JPEG twins
/// for thousands of photos doesn't ask the drive about every possible name (dozens of
/// lookups per photo, which is slow on network drives).
#[derive(Default)]
pub struct DirNames {
    dirs: HashMap<PathBuf, Listing>,
}

#[derive(Default)]
struct Listing {
    /// Lower-case file name -> the name as it is on disk.
    names: HashMap<String, std::ffi::OsString>,
    /// Lower-case stem -> lower-case extensions of the files with that stem.
    stems: HashMap<String, Vec<String>>,
}

impl DirNames {
    fn listing(&mut self, dir: &Path) -> &Listing {
        self.dirs.entry(dir.to_path_buf()).or_insert_with(|| {
            let mut l = Listing::default();
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let name = e.file_name();
                let lower = name.to_string_lossy().to_lowercase();
                let pb = Path::new(&lower);
                let stem = pb.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = pb.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                l.stems.entry(stem).or_default().push(ext);
                l.names.insert(lower, name);
            }
            l
        })
    }

    /// The file in `dir` called `name` (in any case), if there is one.
    fn find(&mut self, dir: &Path, name: &str) -> Option<PathBuf> {
        let l = self.listing(dir);
        l.names.get(&name.to_lowercase()).map(|n| dir.join(n))
    }

    /// The same as [`find_sidecar`], from the folder listing.
    pub fn sidecar(&mut self, p: &Path) -> Option<PathBuf> {
        let dir = p.parent()?;
        let file = p.file_name()?.to_string_lossy().into_owned();
        let stem = p.file_stem()?.to_string_lossy().into_owned();
        let owns_stem = metadata::is_raw(&ext_of(p)) || metadata::video_needs_sidecar(&ext_of(p));
        let full = self.find(dir, &format!("{file}.xmp"));
        if !owns_stem && full.is_some() {
            return full;
        }
        if let Some(st) = self.find(dir, &format!("{stem}.xmp")) {
            if owns_stem || !self.has_sibling(p, true) {
                return Some(st);
            }
        }
        full
    }

    fn has_sibling(&mut self, p: &Path, owners_only: bool) -> bool {
        let (Some(dir), Some(stem)) = (p.parent(), p.file_stem()) else { return false };
        let me = ext_of(p);
        let stem = stem.to_string_lossy().to_lowercase();
        let l = self.listing(dir);
        l.stems.get(&stem).is_some_and(|exts| {
            exts.iter().any(|e| {
                *e != me
                    && metadata::is_supported(e)
                    && (!owners_only || metadata::is_raw(e) || metadata::video_needs_sidecar(e))
            })
        })
    }
}

/// Size of the photo, and the newest modification time of the photo and its XMP sidecar.
/// The catalogue stores this time, so an edit to either file (by Lumio or another app)
/// is noticed, and a photo Lumio just edited isn't indexed again for nothing.
pub fn stat_with_sidecar(p: &Path) -> Option<(i64, i64)> {
    let (size, mtime) = stat(p)?;
    Some((size, mtime.max(sidecar_mtime(find_sidecar(p).as_deref()))))
}

fn sidecar_mtime(side: Option<&Path>) -> i64 {
    side.and_then(stat).map(|(_, sm)| sm).unwrap_or(0)
}

fn stat(p: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    if !m.is_file() {
        return None;
    }
    let mtime =
        m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
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
    // Failed attempts per root whose drive stopped answering, and roots with a retry queued.
    let attempts: parking_lot::Mutex<HashMap<i64, usize>> = Default::default();
    let pending: Arc<parking_lot::Mutex<HashSet<i64>>> = Default::default();
    let notice: parking_lot::Mutex<String> = Default::default();
    let on_err = |e: anyhow::Error, what: &str| {
        let Some(off) = e.downcast_ref::<Offline>() else {
            log::warn!("{what} failed: {e:#}");
            return false;
        };
        let n = {
            let mut a = attempts.lock();
            let n = a.entry(off.root_id).or_insert(0);
            *n += 1;
            *n
        };
        let mins = [1u64, 2, 5, 10, 15][(n - 1).min(4)];
        log::warn!("{off}; indexing paused, retrying in {mins} min");
        let msg = format!(
            "{} isn't reachable. Lumio will carry on when it's back (next try in {mins} min).",
            off.root.display()
        );
        emit_progress(&state, false, 0, 0, &msg);
        *notice.lock() = msg;
        if pending.lock().insert(off.root_id) {
            let st = state.clone();
            let pend = pending.clone();
            let id = off.root_id;
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(mins * 60));
                pend.lock().remove(&id);
                st.queue(Job::ScanRoot(id));
            });
        }
        true
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
        let mut previews = false;
        for j in jobs {
            match j {
                Job::Files(f) => files.extend(f),
                Job::Refresh(f) => refresh.extend(f),
                Job::Removed(f) => removed.extend(f),
                Job::Previews => previews = true,
                Job::ApplyPending => {}
                other => scans.push(other),
            }
        }
        // Moves made while a drive was away go first, so scans find files where the
        // catalogue already says they are.
        if crate::pending::count(&conn) > 0 {
            if let Err(e) = crate::pending::apply(&state, &conn) {
                log::warn!("waiting moves: {e:#}");
            }
        }
        // Anything that indexed files may have left previews to make.
        previews |= !scans.is_empty() || !files.is_empty() || !refresh.is_empty();
        if !removed.is_empty() {
            if let Err(e) = handle_removed(&state, &conn, &removed) {
                log::warn!("remove: {e}");
            }
        }
        let _awake = KeepAwake::new();
        let mut paused = false;
        for s in scans {
            let (r, root) = match s {
                Job::ScanRoot(id) => (scan_root(&state, &conn, id), Some(id)),
                Job::ScanFolder(p) => (scan_folder(&state, &conn, &p), None),
                _ => (Ok(()), None),
            };
            match r {
                Ok(()) => {
                    if let Some(id) = root {
                        attempts.lock().remove(&id);
                    }
                }
                Err(e) => paused |= on_err(e, "scan"),
            }
        }
        if !files.is_empty() {
            if let Err(e) = index_paths(&state, &conn, dedup(files), false) {
                paused |= on_err(e, "indexing");
            }
        }
        if !refresh.is_empty() {
            if let Err(e) = index_paths(&state, &conn, dedup(refresh), true) {
                paused |= on_err(e, "refresh");
            }
        }
        // Offline previews for photos indexed earlier: a few at a time, so other work
        // queued meanwhile goes first.
        let mut more_previews = false;
        if previews && !paused {
            match crate::offline::backfill_chunk(&state, &conn) {
                Ok(more) => more_previews = more,
                Err(e) => paused |= on_err(e, "offline previews"),
            }
            if more_previews {
                state.queue(Job::Previews);
            }
        }
        // Keep the "not reachable" notice up while a retry is waiting.
        if more_previews {
            // Still busy; the next chunk updates the status.
        } else if pending.lock().is_empty() {
            if !paused {
                notice.lock().clear();
                emit_progress(&state, false, 0, 0, "");
            }
        } else {
            emit_progress(&state, false, 0, 0, &notice.lock());
        }
        let _ = state.app.emit("library-changed", ());
    }
}

fn dedup(v: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    v.into_iter().filter(|p| seen.insert(p.clone())).collect()
}

fn emit_progress(state: &AppState, active: bool, done: usize, total: usize, msg: &str) {
    let _ = state.app.emit("index-progress", Progress { active, done, total, message: msg.to_string() });
}

fn handle_removed(state: &AppState, conn: &Connection, paths: &[PathBuf]) -> Result<()> {
    for p in paths {
        if p.exists() {
            continue;
        }
        // If the whole drive or library folder has gone, it's offline, not deleted.
        match db::root_for_path(conn, p)? {
            Some((_, root)) if !reachable(&root) => {
                log::info!("{} went away with its library folder; keeping it in the catalogue", p.display());
                continue;
            }
            _ => {}
        }
        let ps = p.to_string_lossy().to_string();
        let mut args = vec![];
        let clause = db::folder_clause(&ps, true, &mut args);
        args.push(ps.clone().into());
        let sql = format!("SELECT id FROM photos WHERE path = ?{} OR {clause}", args.len());
        let waiting = crate::pending::photo_ids(conn)?;
        let ids: Vec<i64> = {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<i64>>>()?.into_iter().filter(|id| !waiting.contains(id)).collect()
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
        let _ = std::fs::remove_file(thumbs::offline_preview_path(&state.offline_dir, *id));
    }
    db::tidy_stacks(&tx)?;
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
    conn.execute("UPDATE roots SET last_scan = ?1 WHERE id = ?2", params![crate::db::now_ms(), root_id])?;
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
    let mut walk_errors = 0usize;
    for e in walker {
        match e {
            Ok(e) => {
                if e.file_type().is_file() && metadata::is_supported(&ext_of(e.path())) {
                    found.push(e.into_path());
                }
            }
            Err(err) => {
                walk_errors += 1;
                log::warn!("cannot read {}: {err}", err.path().map(|p| p.display().to_string()).unwrap_or_default());
            }
        }
    }
    if walk_errors > 0 && !reachable(dir) {
        // The drive went away while we were listing it; don't forget photos we could not see.
        return Err(Offline { root_id: _root_id, root: dir.to_path_buf() }.into());
    }
    // Forget files that no longer exist below this folder (but not photos moved here
    // while a drive was away: their files haven't arrived yet).
    let present: HashSet<String> = found.iter().map(|p| p.to_string_lossy().to_string()).collect();
    let waiting = crate::pending::photo_ids(conn)?;
    let ds = dir.to_string_lossy().to_string();
    let mut args = vec![];
    let clause = db::folder_clause(&ds, true, &mut args);
    let gone: Vec<i64> = {
        let mut stmt = conn.prepare(&format!("SELECT id, path FROM photos WHERE {clause}"))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.filter_map(|r| r.ok())
            .filter(|(id, p)| !present.contains(p) && !waiting.contains(id))
            .map(|(id, _)| id)
            .collect()
    };
    if !gone.is_empty() {
        if walk_errors > 0 {
            log::warn!(
                "{} photos below {} not found, but some folders could not be read; keeping them",
                gone.len(),
                dir.display()
            );
        } else {
            delete_ids(state, conn, &gone)?;
        }
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
    sidecar: Option<PathBuf>,
}

/// How often a long scan tells the window that the library changed.
const TELL_EVERY: Duration = Duration::from_secs(3);

/// Index the given files (skipping ones whose size and mtime are unchanged unless `force`).
pub fn index_paths(state: &AppState, conn: &Connection, paths: Vec<PathBuf>, force: bool) -> Result<()> {
    let roots: Vec<(i64, PathBuf)> = db::roots(conn)?.into_iter().map(|r| (r.id, PathBuf::from(r.path))).collect();
    // Files waiting to be moved are already in the catalogue at their new place.
    let moving = crate::pending::sources(conn)?;
    let previews = crate::offline::RootPolicy::load(conn)?;
    let mut names = DirNames::default();
    let mut todo: Vec<Pending> = vec![];
    for p in paths {
        let ext = ext_of(&p);
        if !metadata::is_supported(&ext) || is_hidden(&p) || moving.contains(p.to_string_lossy().as_ref()) {
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
        // The time covers the sidecar too, so a changed sidecar makes it differ.
        let Some((size, own_mtime)) = stat(&p) else { continue };
        let sidecar = names.sidecar(&p);
        let mtime = own_mtime.max(sidecar_mtime(sidecar.as_deref()));
        let ps = p.to_string_lossy().to_string();
        let existing = db::photo_by_path(conn, &ps)?;
        if let Some((_, s, m)) = existing {
            if !force && s == size && m == mtime {
                let (thumb, tries): (i64, i64) = conn
                    .query_row("SELECT thumb, thumb_tries FROM photos WHERE path = ?1", [&ps], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .unwrap_or((1, 0));
                if thumb != 0 || tries >= MAX_THUMB_TRIES {
                    continue;
                }
            }
        }
        todo.push(Pending { path: p, ext, root_id, size, mtime, existing: existing.map(|e| e.0), sidecar });
    }
    let total = todo.len();
    if total == 0 {
        return Ok(());
    }
    let mut done = 0;
    let mut failed = 0usize;
    let root_path = |id: i64| roots.iter().find(|(r, _)| *r == id).map(|(_, p)| p.clone()).unwrap_or_default();
    let mut last_told = std::time::Instant::now();
    for chunk in todo.chunks(24) {
        // Before each batch, make sure the drive is still there (once per library folder).
        let batch_roots: HashSet<i64> = chunk.iter().map(|c| c.root_id).collect();
        for id in batch_roots {
            let rp = root_path(id);
            if !reachable(&rp) {
                return Err(Offline { root_id: id, root: rp }.into());
            }
        }
        emit_progress(state, true, done, total, &format!("Indexing {} of {}", done + 1, total));
        // 1. Metadata (image files plus any XMP sidecars), in one ExifTool call.
        let mut files: Vec<PathBuf> = chunk.iter().map(|c| c.path.clone()).collect();
        let sidecars: Vec<Option<PathBuf>> = chunk.iter().map(|c| c.sidecar.clone()).collect();
        files.extend(sidecars.iter().flatten().cloned());
        let mut by_path: HashMap<String, Meta> = HashMap::new();
        if state.et.available() {
            match state.et.read_json(&files) {
                Ok(vals) => {
                    for v in vals {
                        if let Some(err) = v.get("Error").and_then(|s| s.as_str()) {
                            log::warn!(
                                "exiftool: {}: {err}",
                                v.get("SourceFile").and_then(|s| s.as_str()).unwrap_or("?")
                            );
                            continue;
                        }
                        if let Some(src) = v.get("SourceFile").and_then(|s| s.as_str()) {
                            by_path.insert(normalize_src(src), metadata::parse(&v));
                        }
                    }
                }
                Err(e) => log::warn!("exiftool read failed: {e}"),
            }
        }
        // Files ExifTool could not read: if we can't read them either, the drive is
        // dropping out. Skip them rather than cataloguing empty entries.
        let mut skip: HashSet<usize> = HashSet::new();
        for (i, c) in chunk.iter().enumerate() {
            let ps = c.path.to_string_lossy().to_string();
            if !by_path.contains_key(&normalize_src(&ps)) && !readable(&c.path) {
                skip.insert(i);
            }
        }
        if !skip.is_empty() {
            let c = &chunk[*skip.iter().next().unwrap()];
            let rp = root_path(c.root_id);
            if !reachable(&rp) || skip.len() == chunk.len() {
                return Err(Offline { root_id: c.root_id, root: rp }.into());
            }
            for i in &skip {
                log::warn!("cannot read {}; skipped", chunk[*i].path.display());
            }
        }
        // 2. Upsert rows.
        let now = crate::db::now_ms();
        let mut ids: Vec<(i64, PathBuf, String, Option<i64>, i64)> = vec![];
        {
            let tx = conn.unchecked_transaction()?;
            for (i, (c, sc)) in chunk.iter().zip(sidecars.iter()).enumerate() {
                if skip.contains(&i) {
                    continue;
                }
                let ps = c.path.to_string_lossy().to_string();
                let got = by_path.get(&normalize_src(&ps)).cloned();
                if got.is_none() {
                    if let Some(id) = c.existing {
                        // Keep what we already know; just try the thumbnail again.
                        let o: Option<i64> = tx
                            .query_row("SELECT orientation FROM photos WHERE id = ?1", [id], |r| r.get(0))
                            .unwrap_or(None);
                        ids.push((id, c.path.clone(), c.ext.clone(), o, c.root_id));
                        continue;
                    }
                }
                let mut m = got.unwrap_or_default();
                if let Some(sc) = sc {
                    if let Some(side) = by_path.get(&normalize_src(&sc.to_string_lossy())) {
                        metadata::merge_sidecar(&mut m, side);
                    }
                }
                if metadata::is_video(&c.ext) {
                    metadata::fix_video_size(&mut m);
                }
                let folder = c.path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                let filename = c.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let kw = db::encode_keywords(&m.keywords);
                let id = if let Some(id) = c.existing {
                    tx.execute(
                        "UPDATE photos SET root_id=?2, folder=?3, filename=?4, ext=?5, size=?6, mtime=?7, width=?8, height=?9, orientation=?10, taken_at=?11, rating=?12, label=?13, title=?14, caption=?15, keywords=?16, creator=?17, copyright=?18, lat=?19, lon=?20, alt=?21, place=?22, make=?23, model=?24, lens=?25, iso=?26, fnumber=?27, exposure=?28, focal=?29, duration=?30 WHERE id=?1",
                        params![id, c.root_id, folder, filename, c.ext, c.size, c.mtime, m.width, m.height, m.orientation, m.taken_at, m.rating, m.label, m.title, m.caption, kw, m.creator, m.copyright, m.lat, m.lon, m.alt, m.place, m.make, m.model, m.lens, m.iso, m.fnumber, m.exposure, m.focal, m.duration],
                    )?;
                    id
                } else {
                    tx.execute(
                        "INSERT INTO photos (root_id, path, folder, filename, ext, size, mtime, width, height, orientation, taken_at, rating, label, title, caption, keywords, creator, copyright, lat, lon, alt, place, make, model, lens, iso, fnumber, exposure, focal, indexed_at, duration)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31)",
                        params![c.root_id, ps, folder, filename, c.ext, c.size, c.mtime, m.width, m.height, m.orientation, m.taken_at, m.rating, m.label, m.title, m.caption, kw, m.creator, m.copyright, m.lat, m.lon, m.alt, m.place, m.make, m.model, m.lens, m.iso, m.fnumber, m.exposure, m.focal, now, m.duration],
                    )?;
                    tx.last_insert_rowid()
                };
                ids.push((id, c.path.clone(), c.ext.clone(), m.orientation, c.root_id));
            }
            tx.commit()?;
        }
        // 3. Thumbnails, in parallel.
        // A failure only counts against the file if the file itself is still readable.
        // Photos on drives that can go away also get a sharper preview from the same decode.
        let results: Vec<(i64, Result<thumbs::Made, bool>, Option<u32>)> = ids
            .par_iter()
            .map(|(id, path, ext, orient, root_id)| {
                let dest = thumbs::thumb_path(&state.thumbs_dir, *id);
                let size = previews.size_for(*root_id, ext);
                let pdest = size.map(|_| thumbs::offline_preview_path(&state.offline_dir, *id));
                let preview = pdest.as_deref().zip(size);
                match thumbs::make_thumb(path, ext, *orient, &dest, preview, &state.et, &state.tmp_dir) {
                    Ok(made) => (*id, Ok(made), size),
                    Err(e) => {
                        let ok = readable(path);
                        log::warn!(
                            "thumbnail failed for {}: {e:#}{}",
                            path.display(),
                            if ok { "" } else { " (file not readable)" }
                        );
                        (*id, Err(ok), size)
                    }
                }
            })
            .collect();
        let mut unreadable = 0;
        {
            let tx = conn.unchecked_transaction()?;
            for (id, r, size) in &results {
                match r {
                    Ok(made) => {
                        // The file changed (or is new): its old preview, if any, is stale.
                        let preview = match made.preview {
                            Some(true) => size.map(|s| s as i64).unwrap_or(0),
                            Some(false) => -1,
                            None => {
                                let _ = std::fs::remove_file(thumbs::offline_preview_path(&state.offline_dir, *id));
                                0
                            }
                        };
                        tx.execute(
                            "UPDATE photos SET thumb = 1, thumb_tries = 0, preview = ?4, width = coalesce(width, ?2), height = coalesce(height, ?3) WHERE id = ?1",
                            params![id, made.width, made.height, preview],
                        )?
                    }
                    Err(true) => {
                        failed += 1;
                        tx.execute("UPDATE photos SET thumb = 0, thumb_tries = thumb_tries + 1 WHERE id = ?1", [id])?
                    }
                    Err(false) => {
                        unreadable += 1;
                        tx.execute("UPDATE photos SET thumb = 0 WHERE id = ?1", [id])?
                    }
                };
            }
            tx.commit()?;
        }
        if unreadable > 0 {
            let c = &chunk[0];
            let rp = root_path(c.root_id);
            if !reachable(&rp) || unreadable == results.len() {
                let _ = state.app.emit("library-changed", ());
                return Err(Offline { root_id: c.root_id, root: rp }.into());
            }
        }
        done += chunk.len();
        // The window reloads what it shows on this, so during a long scan it is told every
        // few seconds rather than after every batch (and always after the last one).
        if done == total || last_told.elapsed() >= TELL_EVERY {
            last_told = std::time::Instant::now();
            let _ = state.app.emit("library-changed", ());
        }
    }
    log::info!("indexed {total} files, {failed} without a thumbnail");
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

#[cfg(test)]
mod sidecar_tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        tempfile::Builder::new().prefix(&format!("lumio-sidecar-{name}-")).tempdir().unwrap().keep()
    }

    #[test]
    fn the_folder_listing_finds_the_same_sidecars() {
        let d = dir("listing");
        for f in [
            "IMG_1.CR2",
            "IMG_1.JPG",
            "IMG_1.xmp",
            "A.jpg",
            "A.xmp",
            "B.heic",
            "B.mov",
            "B.mov.xmp",
            "C.JPG",
            "C.JPG.xmp",
            "D.dng",
            "D.XMP",
            "2024.07 Iceland.jpg",
        ] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let mut names = DirNames::default();
        for f in ["IMG_1.CR2", "IMG_1.JPG", "A.jpg", "B.heic", "B.mov", "C.JPG", "D.dng", "2024.07 Iceland.jpg"] {
            let p = d.join(f);
            let want = find_sidecar(&p).map(|s| s.to_string_lossy().to_lowercase());
            let got = names.sidecar(&p).map(|s| s.to_string_lossy().to_lowercase());
            assert_eq!(got, want, "{f}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn jpeg_does_not_take_its_raw_twins_sidecar() {
        let d = dir("pair");
        for f in ["IMG_1.CR2", "IMG_1.JPG", "IMG_1.xmp"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        assert_eq!(find_sidecar(&d.join("IMG_1.CR2")), Some(d.join("IMG_1.xmp")));
        assert_eq!(find_sidecar(&d.join("IMG_1.JPG")), None);
        assert_eq!(sidecar_path(&d.join("IMG_1.JPG")), d.join("IMG_1.JPG.xmp"));
        std::fs::write(d.join("IMG_1.JPG.xmp"), b"x").unwrap();
        assert_eq!(find_sidecar(&d.join("IMG_1.JPG")), Some(d.join("IMG_1.JPG.xmp")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn lone_jpeg_and_live_photo() {
        let d = dir("lone");
        std::fs::write(d.join("A.jpg"), b"x").unwrap();
        assert_eq!(sidecar_path(&d.join("A.jpg")), d.join("A.xmp"));
        std::fs::write(d.join("A.xmp"), b"x").unwrap();
        assert_eq!(find_sidecar(&d.join("A.jpg")), Some(d.join("A.xmp")));
        // A Live Photo: the photo and its video each get their own sidecar.
        std::fs::write(d.join("B.heic"), b"x").unwrap();
        std::fs::write(d.join("B.mov"), b"x").unwrap();
        assert_eq!(sidecar_path(&d.join("B.heic")), d.join("B.heic.xmp"));
        assert_eq!(sidecar_path(&d.join("B.mov")), d.join("B.mov.xmp"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
