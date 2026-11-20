use crate::exiftool::ExifTool;
use crate::indexer::Job;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tauri::AppHandle;

pub type Watcher = notify_debouncer_full::Debouncer<notify::RecommendedWatcher, notify_debouncer_full::RecommendedCache>;

pub struct AppState {
    pub app: AppHandle,
    pub db_path: PathBuf,
    pub conn: Mutex<Connection>,
    pub thumbs_dir: PathBuf,
    pub previews_dir: PathBuf,
    /// Sharper previews kept for offline viewing (see `offline.rs`).
    pub offline_dir: PathBuf,
    pub tmp_dir: PathBuf,
    pub et: Arc<ExifTool>,
    pub jobs: Mutex<Sender<Job>>,
    pub watcher: Mutex<Option<Watcher>>,
    pub chat: Mutex<crate::ai::Conversation>,
}

impl AppState {
    pub fn queue(&self, job: Job) {
        let _ = self.jobs.lock().send(job);
    }
}
