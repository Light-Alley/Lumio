//! Sharp previews kept for offline viewing.
//!
//! Thumbnails (480 px) are fine for the grid but look soft in the large view and the
//! filmstrip, and when a drive is unplugged or a network drive is away they are all
//! Lumio had. So for photos on drives that can disconnect, Lumio also keeps a larger
//! JPEG preview in its data folder. It is made while indexing (from the same decoded
//! image as the thumbnail), and for photos indexed before this existed, a background
//! pass fills them in a few at a time whenever the drive is connected.
//!
//! Settings:
//! - `offline_previews`: `removable` (default: external, network and cloud drives),
//!   `all` (every library folder) or `off`.
//! - `offline_preview_size`: `1600` (normal quality, default) or `2560` (high quality).
//!   Changing it remakes existing previews at the new size.
//!
//! `photos.preview` records the size a photo's preview was made at; 0 means none yet,
//! and -1 that it could not be made (tried again when the file changes).

use crate::db;
use crate::indexer::{self, Offline};
use crate::metadata;
use crate::state::AppState;
use crate::thumbs;
use anyhow::Result;
use rayon::prelude::*;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::Emitter;

pub const DEFAULT_SIZE: u32 = 1600;
/// How many previews the background pass makes before letting other work run.
const CHUNK: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Removable,
    All,
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub mode: Mode,
    pub size: u32,
}

impl Policy {
    pub fn load(conn: &Connection) -> Policy {
        let mode = match db::get_setting(conn, "offline_previews").as_deref() {
            Some("off") => Mode::Off,
            Some("all") => Mode::All,
            _ => Mode::Removable,
        };
        let size = db::get_setting(conn, "offline_preview_size")
            .and_then(|s| s.parse().ok())
            .filter(|s| (800..=4096).contains(s))
            .unwrap_or(DEFAULT_SIZE);
        Policy { mode, size }
    }

    /// Should photos below this library folder have an offline preview?
    pub fn wants(&self, root: &Path) -> bool {
        match self.mode {
            Mode::Off => false,
            Mode::All => true,
            Mode::Removable => can_disconnect(root),
        }
    }
}

/// Library roots that want previews, with the answer worked out once per root.
pub struct RootPolicy {
    pub policy: Policy,
    wanted: HashMap<i64, bool>,
}

impl RootPolicy {
    pub fn load(conn: &Connection) -> Result<RootPolicy> {
        let policy = Policy::load(conn);
        let mut wanted = HashMap::new();
        for r in db::list_roots(conn)? {
            // An offline root can't be checked (or read), so it has nothing to make now.
            wanted.insert(r.id, r.online && policy.wants(Path::new(&r.path)));
        }
        Ok(RootPolicy { policy, wanted })
    }

    /// The preview size to make for a photo in this root, if any.
    pub fn size_for(&self, root_id: i64, ext: &str) -> Option<u32> {
        if metadata::is_video(ext) || !self.wanted.get(&root_id).copied().unwrap_or(false) {
            return None;
        }
        Some(self.policy.size)
    }
}

/// Is this folder on a drive that can go away: an external or network drive, or a
/// cloud drive mounted as its own file system (pCloud, Google Drive and the like)?
/// Folders on the computer's own disk (where the home folder is) are not.
pub fn can_disconnect(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let s = path.to_string_lossy();
        if cfg!(target_os = "macos") && s.starts_with("/Volumes/") {
            return true;
        }
        if cfg!(target_os = "linux") && ["/media/", "/mnt/", "/run/media/"].iter().any(|p| s.starts_with(p)) {
            return true;
        }
        let Some(home) = std::env::var_os("HOME") else { return true };
        match (std::fs::metadata(path), std::fs::metadata(home)) {
            (Ok(a), Ok(b)) => a.dev() != b.dev(),
            _ => true,
        }
    }
    #[cfg(windows)]
    {
        let s = path.to_string_lossy().to_lowercase();
        if s.starts_with(r"\\") {
            return true; // network share
        }
        let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_lowercase();
        !s.starts_with(&sys)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        true
    }
}

/// Make the next few missing previews. Returns true if more are waiting.
pub fn backfill_chunk(state: &AppState, conn: &Connection) -> Result<bool> {
    let rp = RootPolicy::load(conn)?;
    if rp.policy.mode == Mode::Off {
        return Ok(false);
    }
    let roots: Vec<(i64, PathBuf)> = db::roots(conn)?
        .into_iter()
        .filter(|r| rp.wanted.get(&r.id).copied().unwrap_or(false))
        .map(|r| (r.id, PathBuf::from(r.path)))
        .collect();
    if roots.is_empty() {
        return Ok(false);
    }
    let list: Vec<String> = roots.iter().map(|(id, _)| id.to_string()).collect();
    let videos: Vec<String> = metadata::VIDEO_EXTS.iter().map(|e| format!("'{e}'")).collect();
    let mut todo: Vec<(i64, PathBuf, String, Option<i64>, i64)> = {
        let mut st = conn.prepare(&format!(
            "SELECT id, path, ext, orientation, root_id FROM photos
             WHERE thumb = 1 AND preview >= 0 AND preview <> ?1 AND root_id IN ({}) AND ext NOT IN ({})
               AND id NOT IN (SELECT photo_id FROM pending_moves)
             ORDER BY coalesce(taken_at, '') DESC, id DESC LIMIT ?2",
            list.join(","),
            videos.join(",")
        ))?;
        let rows = st.query_map(params![rp.policy.size, CHUNK as i64 + 1], |r| {
            Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?), r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let more = todo.len() > CHUNK;
    todo.truncate(CHUNK);
    if todo.is_empty() {
        return Ok(false);
    }
    for (_, _, _, _, root_id) in &todo {
        let root = roots.iter().find(|(id, _)| id == root_id).map(|(_, p)| p.clone()).unwrap_or_default();
        if !indexer::reachable(&root) {
            return Err(Offline { root_id: *root_id, root }.into());
        }
    }
    let (left, _) = counts(conn, &rp)?;
    let _ = state.app.emit(
        "index-progress",
        indexer::Progress { active: true, done: 0, total: 0, message: format!("Saving previews for offline viewing ({} to go)", left) },
    );
    let size = rp.policy.size;
    let results: Vec<(i64, Result<(), bool>)> = todo
        .par_iter()
        .map(|(id, path, ext, orient, _)| {
            let dest = thumbs::offline_preview_path(&state.offline_dir, *id);
            match thumbs::make_offline_preview(path, ext, *orient, &dest, size, &state.et, &state.tmp_dir) {
                Ok(()) => (*id, Ok(())),
                Err(e) => {
                    let readable = indexer::readable(path);
                    log::warn!("offline preview failed for {}: {e:#}", path.display());
                    (*id, Err(readable))
                }
            }
        })
        .collect();
    let mut unreadable = 0;
    {
        let tx = conn.unchecked_transaction()?;
        for (id, r) in &results {
            match r {
                Ok(()) => tx.execute("UPDATE photos SET preview = ?2 WHERE id = ?1", params![id, size])?,
                Err(true) => tx.execute("UPDATE photos SET preview = -1 WHERE id = ?1", [id])?,
                Err(false) => {
                    unreadable += 1;
                    0
                }
            };
        }
        tx.commit()?;
    }
    if unreadable > 0 {
        // The drive is dropping out: stop and let the indexer's retry pick it up later.
        let root_id = todo[0].4;
        let root = roots.iter().find(|(id, _)| *id == root_id).map(|(_, p)| p.clone()).unwrap_or_default();
        if unreadable == results.len() || !indexer::reachable(&root) {
            return Err(Offline { root_id, root }.into());
        }
    }
    Ok(more)
}

/// (still to make, already saved) for the roots that want previews and are connected.
fn counts(conn: &Connection, rp: &RootPolicy) -> Result<(i64, i64)> {
    let ids: Vec<String> = rp.wanted.iter().filter(|(_, w)| **w).map(|(id, _)| id.to_string()).collect();
    if ids.is_empty() {
        return Ok((0, 0));
    }
    let videos: Vec<String> = metadata::VIDEO_EXTS.iter().map(|e| format!("'{e}'")).collect();
    Ok(conn.query_row(
        &format!(
            "SELECT coalesce(SUM(preview >= 0 AND preview <> ?1), 0), coalesce(SUM(preview = ?1), 0) FROM photos
             WHERE thumb = 1 AND root_id IN ({}) AND ext NOT IN ({})",
            ids.join(","),
            videos.join(",")
        ),
        [rp.policy.size],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

#[derive(Debug, Serialize)]
pub struct Stats {
    /// Previews saved on disk (all drives).
    pub saved: i64,
    /// Still to make on connected drives that want them.
    pub waiting: i64,
    /// Disk space the saved previews use, in bytes.
    pub bytes: u64,
    /// Library folders that get previews with the current setting, by name.
    pub drives: Vec<String>,
}

pub fn stats(state: &AppState) -> Result<Stats> {
    let (waiting, saved, drives) = {
        let conn = state.conn.lock();
        let rp = RootPolicy::load(&conn)?;
        let (waiting, _) = counts(&conn, &rp)?;
        let saved: i64 = conn.query_row("SELECT COUNT(*) FROM photos WHERE preview > 0", [], |r| r.get(0))?;
        let drives = db::roots(&conn)?
            .into_iter()
            .filter(|r| rp.wanted.get(&r.id).copied().unwrap_or(false))
            .map(|r| r.name)
            .collect();
        (waiting, saved, drives)
    };
    Ok(Stats { saved, waiting, bytes: dir_size(&state.offline_dir), drives })
}

fn dir_size(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Delete every saved preview (they are made again if the setting still asks for them).
pub fn clear(state: &AppState) -> Result<()> {
    state.conn.lock().execute("UPDATE photos SET preview = 0", [])?;
    if state.offline_dir.exists() {
        std::fs::remove_dir_all(&state.offline_dir)?;
    }
    std::fs::create_dir_all(&state.offline_dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_folder_is_not_removable() {
        if let Some(home) = std::env::var_os("HOME") {
            assert!(!can_disconnect(Path::new(&home)));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn volumes_are_removable() {
        assert!(can_disconnect(Path::new("/Volumes/Photos")));
    }

    #[test]
    fn missing_folder_counts_as_removable() {
        assert!(can_disconnect(Path::new("/definitely/not/here/lumio")));
    }
}
