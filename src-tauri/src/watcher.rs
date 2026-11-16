//! Watches library folders so files saved or exported by other apps appear automatically.

use crate::db;
use crate::indexer::{ext_of, is_hidden, Job};
use crate::metadata;
use crate::state::AppState;
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub fn restart(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
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
                if p.ancestors().any(|a| a != p.as_path() && is_hidden(a)) || is_hidden(p) {
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
                } else if !p.exists() && ext.is_empty() {
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
    let roots: Vec<PathBuf> = {
        let conn = state.conn.lock();
        db::list_roots(&conn).map(|r| r.into_iter().filter(|r| r.online).map(|r| PathBuf::from(r.path)).collect()).unwrap_or_default()
    };
    for r in roots {
        if let Err(e) = debouncer.watch(&r, RecursiveMode::Recursive) {
            log::warn!("cannot watch {}: {e}", r.display());
        }
    }
    *state.watcher.lock() = Some(debouncer);
}
