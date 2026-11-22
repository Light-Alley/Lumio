//! Tell the system file manager (Finder / File Explorer) that a folder changed, so open
//! windows refresh straight away. Virtual drives such as pCloud, Google Drive or SMB
//! shares often don't announce changes made by other apps on their own.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The app, for running on its main thread (set once at start-up).
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

pub fn init(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

/// Notify about changes in these folders (and the folders containing these files).
pub fn changed<P: AsRef<Path>>(paths: impl IntoIterator<Item = P>) {
    let mut dirs: HashSet<PathBuf> = HashSet::new();
    for p in paths {
        let p = p.as_ref();
        dirs.insert(p.to_path_buf());
        if let Some(parent) = p.parent() {
            dirs.insert(parent.to_path_buf());
        }
    }
    let dirs: Vec<PathBuf> = dirs.into_iter().filter(|d| d.exists()).collect();
    if dirs.is_empty() {
        return;
    }
    imp::notify(&dirs);
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;
    use std::path::PathBuf;

    /// Finder still refreshes on `noteFileSystemChanged:`, though it is no longer
    /// documented. AppKit objects belong on the main thread, so the call goes there
    /// (callers are worker threads), and nothing waits for it.
    pub fn notify(dirs: &[PathBuf]) {
        let Some(app) = super::APP.get() else { return };
        let dirs: Vec<String> = dirs.iter().map(|d| d.to_string_lossy().into_owned()).collect();
        let _ = app.run_on_main_thread(move || unsafe {
            let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            if ws.is_null() {
                return;
            }
            for d in &dirs {
                let s = NSString::from_str(d);
                let _: () = msg_send![ws, noteFileSystemChanged: &*s];
            }
        });
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_UPDATEDIR, SHCNF_FLUSHNOWAIT, SHCNF_PATHW};

    pub fn notify(dirs: &[PathBuf]) {
        for d in dirs {
            let wide: Vec<u16> = d.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            unsafe {
                SHChangeNotify(SHCNE_UPDATEDIR, SHCNF_PATHW | SHCNF_FLUSHNOWAIT, Some(wide.as_ptr() as *const _), None);
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    // Linux file managers watch folders themselves via inotify.
    pub fn notify(_dirs: &[std::path::PathBuf]) {}
}
