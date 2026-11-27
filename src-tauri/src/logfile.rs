//! A small file logger, so there is a record of why a file could not be indexed.
//! Writes to `frameloft.log` in the app's data folder and keeps one older copy.

use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

const MAX_BYTES: u64 = 5 * 1024 * 1024;

struct FileLog {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl FileLog {
    fn open(&self) -> Option<File> {
        let mut o = OpenOptions::new();
        o.create(true).append(true);
        // Only the owner may read the log (it names files and folders).
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        o.open(&self.path).ok()
    }
}

impl log::Log for FileLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info && m.target().starts_with("frameloft")
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:<5} {}\n", jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S"), r.level(), r.args());
        if cfg!(debug_assertions) {
            eprint!("{line}");
        }
        let mut guard = self.file.lock();
        if guard.as_ref().and_then(|f| f.metadata().ok()).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
            *guard = None;
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
        }
        if guard.is_none() {
            *guard = self.open();
        }
        if let Some(f) = guard.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().as_mut() {
            let _ = f.flush();
        }
    }
}

pub fn init(path: PathBuf) {
    let logger = Box::new(FileLog { path, file: Mutex::new(None) });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;

    fn record(log: &FileLog, target: &str, msg: &str) {
        log.log(&log::Record::builder().target(target).level(log::Level::Info).args(format_args!("{msg}")).build());
        log.flush();
    }

    #[test]
    fn writes_only_framelofts_lines_and_keeps_them_private() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("frameloft.log");
        let log = FileLog { path: path.clone(), file: Mutex::new(None) };
        record(&log, "frameloft_lib::indexer", "indexed 3 files");
        record(&log, "hyper::client", "noise from a library");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("indexed 3 files"));
        assert!(!text.contains("noise"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn a_full_log_starts_again_and_keeps_one_old_copy() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("frameloft.log");
        std::fs::write(&path, vec![b'x'; (MAX_BYTES + 10) as usize]).unwrap();
        let log = FileLog { path: path.clone(), file: Mutex::new(None) };
        record(&log, "frameloft", "first");
        record(&log, "frameloft", "second");
        assert!(std::fs::metadata(&path).unwrap().len() < 1000);
        assert!(path.with_extension("log.1").exists());
    }
}
