//! Watches library folders so files saved or exported by other apps appear automatically.

use crate::db;
use crate::indexer::{ext_of, is_hidden, Job};
use crate::metadata;
use crate::state::AppState;
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Whether `p` is inside a hidden folder (or is hidden itself), counting only the folders
/// below the library folder that contains it: a library kept inside a hidden folder
/// (such as `~/.photos/Library`) is still watched.
fn hidden_inside(roots: &[PathBuf], p: &Path) -> bool {
    let root = roots.iter().filter(|r| p.starts_with(r)).max_by_key(|r| r.components().count());
    let rel = match root {
        Some(r) => p.strip_prefix(r).unwrap_or(p),
        None => return true, // not in any library: nothing to do with it
    };
    let mut cur = PathBuf::new();
    for c in rel.components() {
        cur.push(c);
        if is_hidden(&cur) {
            return true;
        }
    }
    false
}

pub fn restart(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    let roots: Vec<PathBuf> = {
        let conn = state.conn.lock();
        db::list_roots(&conn).map(|r| r.into_iter().filter(|r| r.online).map(|r| PathBuf::from(r.path)).collect()).unwrap_or_default()
    };
    let watched = roots.clone();
    let debouncer = new_debouncer(Duration::from_millis(1500), None, move |res: DebounceEventResult| {
        let Some(state) = weak.upgrade() else { return };
        let Ok(events) = res else { return };
        let mut files = vec![];
        let mut removed = vec![];
        let mut folders = vec![];
        for ev in events {
            if matches!(ev.kind, EventKind::Access(_)) {
                continue;
            }
            for p in &ev.paths {
                if hidden_inside(&watched, p) {
                    continue;
                }
                let ext = ext_of(p);
                if ext == "xmp" {
                    // A sidecar changed: refresh the photo next to it.
                    if let Some(parent) = p.parent() {
                        let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                        if let Ok(rd) = std::fs::read_dir(parent) {
                            for e in rd.flatten() {
                                let ep = e.path();
                                let es = ep.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                                let full = ep.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                                if (es == stem || full == stem) && metadata::is_supported(&ext_of(&ep)) {
                                    files.push(ep);
                                }
                            }
                        }
                    }
                } else if metadata::is_supported(&ext) {
                    if p.exists() { files.push(p.clone()) } else { removed.push(p.clone()) }
                } else if p.is_dir() {
                    if matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))) {
                        folders.push(p.clone());
                    }
                } else if !p.exists() {
                    // Something else went away. It may have been a folder, whose name can
                    // hold a dot ("2024.07 Iceland"), so it can't be told apart from a file
                    // by its name; the catalogue is asked what was inside it instead.
                    removed.push(p.clone());
                }
            }
        }
        if !removed.is_empty() { state.queue(Job::Removed(removed)); }
        for f in folders { state.queue(Job::ScanFolder(f)); }
        if !files.is_empty() { state.queue(Job::Files(files)); }
    });
    let mut debouncer = match debouncer {
        Ok(d) => d,
        Err(e) => { log::warn!("file watcher unavailable: {e}"); return; }
    };
    for r in roots {
        if let Err(e) = debouncer.watch(&r, RecursiveMode::Recursive) {
            log::warn!("cannot watch {}: {e}", r.display());
        }
    }
    *state.watcher.lock() = Some(debouncer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_inside_a_hidden_folder_is_watched() {
        let roots = vec![PathBuf::from("/home/me/.photos/Library")];
        assert!(!hidden_inside(&roots, Path::new("/home/me/.photos/Library/2024/a.jpg")));
        assert!(hidden_inside(&roots, Path::new("/home/me/.photos/Library/.cache/a.jpg")));
        assert!(hidden_inside(&roots, Path::new("/home/me/.photos/Library/2024/._a.jpg")));
        assert!(hidden_inside(&roots, Path::new("/elsewhere/a.jpg")));
    }

    #[test]
    fn dotted_folders_are_not_hidden() {
        let roots = vec![PathBuf::from("/lib")];
        assert!(!hidden_inside(&roots, Path::new("/lib/2024.07 Iceland")));
        assert!(!hidden_inside(&roots, Path::new("/lib/2024.07 Iceland/a.jpg")));
    }
}
