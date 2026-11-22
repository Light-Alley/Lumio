//! Sharing photos: the system share sheet, email, WhatsApp, websites and the clipboard.
//!
//! Lumio never talks to a social network itself and stores no passwords for them. It hands
//! the photos to something already on the computer (the share sheet, the email program,
//! WhatsApp) or opens the website in the browser with the photos ready to paste or drag in.
//!
//! Formats that websites and phones can't show (RAW, HEIC, TIFF…) are first turned into
//! JPEG copies in Lumio's cache folder; the originals are never touched.

use crate::db::Photo;
use crate::state::AppState;
use crate::{db, metadata, thumbs};
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tauri::Manager;

/// Long side of the "smaller copies" option, a size every website and messenger accepts.
const SMALL_EDGE: u32 = 2048;
/// Largest picture we put on the clipboard; bigger ones make pasting slow.
const CLIPBOARD_EDGE: u32 = 4096;

/// What happened, in words the person can act on. Shown as a message in the app.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub message: String,
}

fn done(msg: impl Into<String>) -> Outcome {
    Outcome { message: msg.into() }
}

pub const SITES: &[(&str, &str, &str)] = &[
    // (key, name, page that starts a new post)
    ("facebook", "Facebook", "https://www.facebook.com/"),
    ("instagram", "Instagram", "https://www.instagram.com/"),
    ("x", "X", "https://x.com/compose/post"),
    ("bluesky", "Bluesky", "https://bsky.app/"),
    ("threads", "Threads", "https://www.threads.com/"),
    ("pinterest", "Pinterest", "https://www.pinterest.com/pin-creation-tool/"),
    ("flickr", "Flickr", "https://www.flickr.com/photos/upload/"),
    ("linkedin", "LinkedIn", "https://www.linkedin.com/feed/?shareActive=true"),
    ("gmail", "Gmail", "https://mail.google.com/mail/?view=cm&fs=1"),
    ("outlook", "Outlook.com", "https://outlook.live.com/mail/0/deeplink/compose"),
];

// ------------------------------------------------------------------ preparing files

fn web_friendly(ext: &str) -> bool {
    matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp")
}

fn is_still_image(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    !metadata::is_video(ext) && web_friendly(ext)
}

fn share_root(state: &AppState) -> Result<PathBuf> {
    Ok(state.app.path().app_cache_dir()?.join("share"))
}

/// Forget copies made for earlier shares. Email programs and the share sheet may read
/// a copy some time after we hand it over, so only folders older than a day go.
fn clean_old(root: &Path) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    let Some(cutoff) = SystemTime::now().checked_sub(Duration::from_secs(24 * 3600)) else { return };
    for e in rd.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).map(|t| t < cutoff).unwrap_or(false);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    p
}

/// The files to hand over for these photos: the originals where any app can open them,
/// otherwise JPEG copies. With the "smaller copies" setting every still photo becomes a
/// 2048-pixel JPEG, which also leaves the camera details and location behind.
pub fn prepare(state: &AppState, ids: &[i64]) -> Result<Vec<PathBuf>> {
    let (photos, small) = {
        let conn = state.conn.lock();
        let photos = db::get_photos(&conn, ids)?;
        let small = db::get_setting(&conn, "share_size").as_deref() == Some("small");
        (photos, small)
    };
    if photos.is_empty() {
        bail!("Select one or more photos first.");
    }
    let root = share_root(state)?;
    clean_old(&root);
    let mut dir: Option<PathBuf> = None;
    let mut out = Vec::with_capacity(photos.len());
    for p in &photos {
        let src = Path::new(&p.path);
        if !src.exists() {
            bail!("{} is not available (is its drive connected?)", p.filename);
        }
        if !small && (metadata::is_video(&p.ext) || web_friendly(&p.ext)) {
            out.push(src.to_path_buf());
            continue;
        }
        let d = match &dir {
            Some(d) => d.clone(),
            None => {
                let stamp =
                    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                let d = root.join(stamp.to_string());
                std::fs::create_dir_all(&d)?;
                dir = Some(d.clone());
                d
            }
        };
        if metadata::is_video(&p.ext) {
            out.push(video_copy(state, p, &d)?);
        } else {
            out.push(convert(state, p, small, &d)?);
        }
    }
    Ok(out)
}

/// "Small copies" promise copies without the original's details. Videos aren't made
/// smaller, but the copy loses its location (coordinates and place names), as the
/// photo copies do (they are re-encoded without any metadata).
fn video_copy(state: &AppState, p: &Photo, dir: &Path) -> Result<PathBuf> {
    let src = Path::new(&p.path);
    let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "video".into());
    let ext = src.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.ext.clone());
    let dest = unique(dir, &stem, &ext);
    std::fs::copy(src, &dest).map_err(|e| anyhow!("Could not copy {}: {e}", p.filename))?;
    if let Err(e) = state.et.write(&dest, &metadata::strip_location_args()) {
        let _ = std::fs::remove_file(&dest);
        bail!("Could not remove the location from {}: {e}", p.filename);
    }
    Ok(dest)
}

fn convert(state: &AppState, p: &Photo, small: bool, dir: &Path) -> Result<PathBuf> {
    let want = if small { SMALL_EDGE } else { u32::MAX };
    let img = thumbs::load_image(Path::new(&p.path), &p.ext, p.orientation, want, &state.et, &state.tmp_dir)
        .map_err(|e| anyhow!("Could not make a JPEG copy of {}: {e}", p.filename))?;
    let img = if small && img.width().max(img.height()) > SMALL_EDGE {
        img.resize(SMALL_EDGE, SMALL_EDGE, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let stem =
        Path::new(&p.filename).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "photo".into());
    let dest = unique(dir, &stem, "jpg");
    std::fs::write(&dest, thumbs::encode_jpeg(&img, 90)?)?;
    Ok(dest)
}

fn count(files: &[PathBuf]) -> String {
    let videos =
        files.iter().filter(|f| metadata::is_video(f.extension().and_then(|e| e.to_str()).unwrap_or(""))).count();
    match (files.len(), videos) {
        (1, 1) => "The video".into(),
        (1, _) => "The photo".into(),
        (n, v) if v == n => format!("{n} videos"),
        (n, 0) => format!("{n} photos"),
        (n, _) => format!("{n} items"),
    }
}

// ------------------------------------------------------------------ clipboard

static CLIPBOARD: parking_lot::Mutex<Option<arboard::Clipboard>> = parking_lot::const_mutex(None);

#[derive(PartialEq)]
pub enum Copied {
    /// The picture itself: pastes into chats, emails and web pages.
    Picture,
    /// The files, as if copied in the file manager.
    Files,
}

/// Put the photos on the clipboard. One photo goes on as a picture, which every app can
/// paste; several photos (or a video) go on as files.
pub fn copy(files: &[PathBuf]) -> Result<Copied> {
    let mut guard = CLIPBOARD.lock();
    if guard.is_none() {
        // On Linux the clipboard only holds what we copied while this object lives,
        // so it is kept for as long as Lumio runs.
        *guard = Some(arboard::Clipboard::new().map_err(|e| anyhow!("Could not use the clipboard: {e}"))?);
    }
    let cb = guard.as_mut().unwrap();
    if files.len() == 1 && is_still_image(&files[0]) {
        // Upright, as the photo is shown (a portrait JPEG must not paste sideways).
        let img = crate::thumbs::open_upright(&files[0])?;
        let img = if img.width().max(img.height()) > CLIPBOARD_EDGE {
            img.resize(CLIPBOARD_EDGE, CLIPBOARD_EDGE, image::imageops::FilterType::Triangle)
        } else {
            img
        };
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        cb.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: rgba.into_raw().into() })
            .map_err(|e| anyhow!("Could not copy the photo: {e}"))?;
        return Ok(Copied::Picture);
    }
    cb.set().file_list(files).map_err(|e| anyhow!("Could not copy the files: {e}"))?;
    Ok(Copied::Files)
}

fn paste_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘V"
    } else {
        "Ctrl+V"
    }
}

fn file_manager() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else if cfg!(target_os = "windows") {
        "File Explorer"
    } else {
        "file manager"
    }
}

fn reveal(files: &[PathBuf]) {
    if let Some(f) = files.first() {
        let _ = tauri_plugin_opener::reveal_item_in_dir(f);
    }
}

fn open_url(url: &str) -> Result<()> {
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| anyhow!("Could not open the browser: {e}"))
}

pub fn copy_only(state: &AppState, ids: &[i64]) -> Result<Outcome> {
    let files = prepare(state, ids)?;
    let what = count(&files);
    match copy(&files)? {
        Copied::Picture => {
            Ok(done(format!("{what} is copied. Paste it with {} into a chat, email or post.", paste_key())))
        }
        Copied::Files => {
            Ok(done(format!("{what} copied. Paste them with {} into a chat, email or folder.", paste_key())))
        }
    }
}

pub fn show_copies(state: &AppState, ids: &[i64]) -> Result<Outcome> {
    let files = prepare(state, ids)?;
    reveal(&files);
    Ok(done(format!("{} shown in {}. Drag into any website or app.", count(&files), file_manager())))
}

// ------------------------------------------------------------------ websites

pub fn site(state: &AppState, ids: &[i64], key: &str) -> Result<Outcome> {
    let &(_, name, url) = SITES.iter().find(|s| s.0 == key).ok_or_else(|| anyhow!("Unknown website"))?;
    let files = prepare(state, ids)?;
    let copied = copy(&files).ok();
    reveal(&files);
    open_url(url)?;
    let how = if copied == Some(Copied::Picture) {
        format!("paste it with {} or drag it in from the {} window", paste_key(), file_manager())
    } else {
        format!("drag them in from the {} window", file_manager())
    };
    Ok(done(format!("{name} is opening in your browser. Start a new post, then {how}.")))
}

// ------------------------------------------------------------------ email

pub fn email(state: &AppState, ids: &[i64]) -> Result<Outcome> {
    let files = prepare(state, ids)?;
    let subject = if files.len() == 1 {
        files[0].file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Photo".into())
    } else {
        format!("{} photos", files.len())
    };
    match imp::email(state, &files, &subject) {
        Ok(()) => Ok(done(format!("{} is attached to a new email.", count(&files)))),
        Err(e) => {
            log::info!("email with attachments failed, using mailto: {e:#}");
            // No email program we can hand attachments to: open a blank message and the
            // files, so they can be dragged in.
            reveal(&files);
            open_url(&format!("mailto:?subject={}", url_escape(&subject)))?;
            Ok(done(format!(
                "Your email program couldn't take the attachments directly. Drag {} in from the {} window.",
                count(&files).to_lowercase(),
                file_manager()
            )))
        }
    }
}

fn url_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ------------------------------------------------------------------ WhatsApp

pub fn whatsapp(state: &AppState, ids: &[i64], web: bool) -> Result<Outcome> {
    let files = prepare(state, ids)?;
    log::info!("share: WhatsApp{} with {} file(s)", if web { " Web" } else { "" }, files.len());
    if !web && imp::whatsapp_installed() {
        // The photo also goes on the clipboard, in case the app doesn't take the files.
        let copied = copy(&files)?;
        if imp::open_whatsapp_with(&files)? {
            return Ok(done(format!(
                "{} is opening in WhatsApp. Choose a chat and press Send. (If it doesn't appear, open a chat and press {}.)",
                count(&files),
                paste_key()
            )));
        }
        return Ok(done(pasted_hint("WhatsApp", copied, &files)));
    }
    let copied = copy(&files)?;
    if copied == Copied::Files {
        reveal(&files);
    }
    open_url("https://web.whatsapp.com/")?;
    let note = if web { "" } else { "WhatsApp isn't installed, so WhatsApp Web is opening in your browser. " };
    Ok(done(format!("{note}{}", pasted_hint("WhatsApp Web", copied, &files))))
}

fn pasted_hint(app: &str, copied: Copied, files: &[PathBuf]) -> String {
    let what = count(files);
    match copied {
        Copied::Picture => format!("{what} is copied. In {app}, open a chat and press {} to paste it.", paste_key()),
        Copied::Files => format!(
            "{what} copied. In {app}, open a chat and press {} to paste, or drag them in from the {} window.",
            paste_key(),
            file_manager()
        ),
    }
}

// ------------------------------------------------------------------ share sheet

/// Whether this computer has a system share sheet (macOS and Windows do; Linux doesn't).
pub fn has_sheet() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Open the system share sheet. `x`, `y` are where the person clicked, in window
/// coordinates, so on macOS the sheet appears next to the pointer.
pub fn sheet(state: &AppState, ids: &[i64], x: f64, y: f64) -> Result<Outcome> {
    if !has_sheet() {
        bail!("This system has no share menu. Use Email, WhatsApp or a website instead.");
    }
    let files = prepare(state, ids)?;
    imp::sheet(state, &files, x, y)?;
    Ok(done(""))
}

/// Run `f` on the main (interface) thread and wait for its answer.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn on_main<T: Send + 'static>(state: &AppState, f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    state.app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })?;
    rx.recv_timeout(Duration::from_secs(20)).map_err(|_| anyhow!("The system did not answer"))?
}

// ---------------------------------------------------------------- macOS
#[cfg(target_os = "macos")]
mod imp {
    use super::on_main;
    use crate::state::AppState;
    use anyhow::{anyhow, bail, Result};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_app_kit::{NSSharingService, NSSharingServiceNameComposeEmail, NSSharingServicePicker, NSView};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
    use std::cell::RefCell;
    use std::path::PathBuf;
    use tauri::Manager;

    thread_local! {
        // The picker must stay alive while it is on screen.
        static PICKER: RefCell<Option<Retained<NSSharingServicePicker>>> = const { RefCell::new(None) };
    }

    fn items(files: &[PathBuf]) -> Retained<NSArray<AnyObject>> {
        let urls: Vec<Retained<NSURL>> =
            files.iter().map(|f| NSURL::fileURLWithPath(&NSString::from_str(&f.to_string_lossy()))).collect();
        let arr = NSArray::from_retained_slice(&urls);
        // SAFETY: an NSArray of NSURLs is an NSArray of objects.
        unsafe { Retained::cast_unchecked(arr) }
    }

    pub fn sheet(state: &AppState, files: &[PathBuf], x: f64, y: f64) -> Result<()> {
        let files = files.to_vec();
        let app = state.app.clone();
        on_main(state, move || {
            // The window and its view are looked up here, on the main thread, so the view
            // can't be closed and freed between the lookup and its use.
            let window = app.get_webview_window("main").ok_or_else(|| anyhow!("no window"))?;
            let view_ptr = window.ns_view()?;
            if view_ptr.is_null() {
                bail!("no window");
            }
            // SAFETY: Tauri gives us the live content view of a window that exists right
            // now, and we are on the main thread, where AppKit objects may be used.
            let view: &NSView = unsafe { &*(view_ptr as *const NSView) };
            let b = view.bounds();
            let y = if view.isFlipped() { y } else { b.size.height - y };
            let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(1.0, 1.0));
            let picker =
                unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items(&files)) };
            picker.showRelativeToRect_ofView_preferredEdge(rect, view, NSRectEdge::MinY);
            PICKER.with(|p| *p.borrow_mut() = Some(picker));
            Ok(())
        })
    }

    pub fn email(state: &AppState, files: &[PathBuf], subject: &str) -> Result<()> {
        let files = files.to_vec();
        let subject = subject.to_string();
        on_main(state, move || {
            let service = NSSharingService::sharingServiceNamed(unsafe { NSSharingServiceNameComposeEmail })
                .ok_or_else(|| anyhow!("Mail is not available"))?;
            let items = items(&files);
            if !unsafe { service.canPerformWithItems(Some(&items)) } {
                bail!("Mail can't send these files");
            }
            service.setSubject(Some(&NSString::from_str(&subject)));
            unsafe { service.performWithItems(&items) };
            Ok(())
        })
    }

    pub fn whatsapp_installed() -> bool {
        std::process::Command::new("/usr/bin/open")
            .args(["-Ra", "WhatsApp"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// WhatsApp for Mac has no share extension, but it opens images and videos handed
    /// to it (it declares `public.image` and `public.movie`), showing a chat picker.
    pub fn open_whatsapp_with(files: &[PathBuf]) -> Result<bool> {
        // Absolute paths can never be read as options of `open`.
        if files.iter().any(|f| !f.is_absolute()) {
            bail!("not a file path");
        }
        let status = std::process::Command::new("/usr/bin/open").args(["-a", "WhatsApp"]).args(files).status()?;
        Ok(status.success())
    }
}

// ---------------------------------------------------------------- Windows
#[cfg(target_os = "windows")]
mod imp {
    use super::on_main;
    use crate::state::AppState;
    use anyhow::{anyhow, bail, Result};
    use std::path::PathBuf;
    use std::time::Duration;
    use tauri::Manager;
    use windows::core::{w, HSTRING, PCWSTR, PWSTR};
    use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
    use windows::Foundation::{Collections::IIterable, EventRegistrationToken, TypedEventHandler};
    use windows::Storage::{IStorageItem, StorageFile};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::System::Mapi::{MapiFileDescW, MapiMessageW};
    use windows::Win32::UI::Shell::{
        AssocQueryStringW, IDataTransferManagerInterop, ASSOCF_IS_PROTOCOL, ASSOCSTR_FRIENDLYAPPNAME,
    };

    static TOKEN: parking_lot::Mutex<Option<(isize, i64)>> = parking_lot::const_mutex(None);

    /// Storage items, carried to the interface thread. Only this one type is marked Send
    /// (not any T): StorageFile objects are "agile" WinRT objects (they implement
    /// IAgileObject), which any thread may use, but the bindings don't say so.
    struct AgileItems(Vec<Option<IStorageItem>>);
    // SAFETY: see above; IStorageItem from StorageFile is agile, so moving the references
    // to another thread and using them there is allowed.
    unsafe impl Send for AgileItems {}

    /// Undoes CoInitializeEx when dropped, if it succeeded.
    struct ComGuard(bool);
    impl ComGuard {
        fn sta() -> ComGuard {
            ComGuard(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
        }
    }
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe { windows::Win32::System::Com::CoUninitialize() };
            }
        }
    }

    /// Frees a loaded DLL when dropped.
    struct Library(windows::Win32::Foundation::HMODULE);
    impl Drop for Library {
        fn drop(&mut self) {
            let _ = unsafe { windows::Win32::Foundation::FreeLibrary(self.0) };
        }
    }

    fn hwnd(state: &AppState) -> Result<isize> {
        let window = state.app.get_webview_window("main").ok_or_else(|| anyhow!("no window"))?;
        Ok(window.hwnd()?.0 as isize)
    }

    pub fn sheet(state: &AppState, files: &[PathBuf], _x: f64, _y: f64) -> Result<()> {
        // Look the files up before showing the sheet: the lookup waits on the disk,
        // which must not happen on the interface thread.
        let storage: AgileItems = {
            // Joins this worker thread to the process-wide multithreaded apartment. It is
            // deliberately not left again: the share sheet uses these items after this
            // function returns, and leaving the last MTA thread would tear it down.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let mut v = vec![];
            for f in files {
                let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(f.as_os_str()))?.get()?;
                v.push(Some(windows::core::Interface::cast::<IStorageItem>(&file)?));
            }
            AgileItems(v)
        };
        let count = files.len();
        let h = hwnd(state)?;
        on_main(state, move || unsafe {
            let storage = storage;
            let hwnd = HWND(h as *mut _);
            let interop = windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()?;
            let dtm: DataTransferManager = interop.GetForWindow(hwnd)?;
            if let Some((old_h, t)) = TOKEN.lock().take() {
                if old_h == h {
                    let _ = dtm.RemoveDataRequested(EventRegistrationToken { Value: t });
                }
            }
            let storage = AgileItems(storage.0);
            let token =
                dtm.DataRequested(&TypedEventHandler::new(move |_, args: &Option<DataRequestedEventArgs>| {
                    let Some(args) = args else { return Ok(()) };
                    let data = args.Request()?.Data()?;
                    let title =
                        if count == 1 { "Photo from Lumio".to_string() } else { format!("{count} photos from Lumio") };
                    data.Properties()?.SetTitle(&HSTRING::from(title))?;
                    let list: IIterable<IStorageItem> = {
                        let s = &storage; // the whole wrapper, so the closure stays Send
                        IIterable::try_from(s.0.clone())?
                    };
                    data.SetStorageItemsReadOnly(&list)?;
                    Ok(())
                }))?;
            *TOKEN.lock() = Some((h, token.Value));
            interop.ShowShareUIForWindow(hwnd)?;
            Ok(())
        })
    }

    type SendMailW = unsafe extern "system" fn(usize, usize, *const MapiMessageW, u32, u32) -> u32;
    const MAPI_LOGON_UI: u32 = 0x1;
    const MAPI_DIALOG: u32 = 0x8;
    const MAPI_USER_ABORT: u32 = 1;

    /// Hand the files to the email program (Outlook, Thunderbird…) through Simple MAPI.
    pub fn email(state: &AppState, files: &[PathBuf], subject: &str) -> Result<()> {
        let h = hwnd(state)? as usize;
        let files = files.to_vec();
        let subject = subject.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        // The email window can stay open (and this call with it) until the message is
        // sent, so it gets its own thread.
        std::thread::spawn(move || {
            let r = (|| -> Result<()> {
                let _com = ComGuard::sta();
                unsafe {
                    let lib =
                        Library(LoadLibraryW(w!("MAPI32.DLL")).map_err(|_| anyhow!("no email program is set up"))?);
                    let f = GetProcAddress(lib.0, windows::core::s!("MAPISendMailW"))
                        .ok_or_else(|| anyhow!("no MAPISendMailW"))?;
                    let send: SendMailW = std::mem::transmute(f);
                    let mut paths: Vec<Vec<u16>> = files.iter().map(|f| wide(&f.to_string_lossy())).collect();
                    let mut names: Vec<Vec<u16>> = files
                        .iter()
                        .map(|f| wide(&f.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()))
                        .collect();
                    let mut descs: Vec<MapiFileDescW> = paths
                        .iter_mut()
                        .zip(names.iter_mut())
                        .map(|(p, n)| MapiFileDescW {
                            ulReserved: 0,
                            flFlags: 0,
                            nPosition: u32::MAX,
                            lpszPathName: PWSTR(p.as_mut_ptr()),
                            lpszFileName: PWSTR(n.as_mut_ptr()),
                            lpFileType: std::ptr::null_mut(),
                        })
                        .collect();
                    let mut subj = wide(&subject);
                    let msg = MapiMessageW {
                        lpszSubject: PWSTR(subj.as_mut_ptr()),
                        nFileCount: descs.len() as u32,
                        lpFiles: descs.as_mut_ptr(),
                        ..Default::default()
                    };
                    let rc = send(0, h, &msg, MAPI_LOGON_UI | MAPI_DIALOG, 0);
                    if rc != 0 && rc != MAPI_USER_ABORT {
                        bail!("MAPI error {rc}");
                    }
                    Ok(())
                }
            })();
            let _ = tx.send(r);
        });
        match rx.recv_timeout(Duration::from_secs(4)) {
            Ok(r) => r,
            // Still waiting: the email window is open.
            Err(_) => Ok(()),
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn whatsapp_installed() -> bool {
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        unsafe {
            AssocQueryStringW(
                ASSOCF_IS_PROTOCOL,
                ASSOCSTR_FRIENDLYAPPNAME,
                w!("whatsapp"),
                PCWSTR::null(),
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok()
        }
    }

    /// Windows: WhatsApp is opened and the photo pasted from the clipboard.
    pub fn open_whatsapp_with(_files: &[PathBuf]) -> Result<bool> {
        tauri_plugin_opener::open_url("whatsapp://", None::<&str>).map_err(|e| anyhow!("{e}"))?;
        Ok(false)
    }
}

// ---------------------------------------------------------------- Linux / BSD
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use crate::state::AppState;
    use anyhow::{bail, Result};
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    pub fn sheet(_state: &AppState, _files: &[PathBuf], _x: f64, _y: f64) -> Result<()> {
        bail!("not available on this system")
    }

    /// xdg-email passes attachments to Thunderbird, Evolution, KMail and others.
    pub fn email(_state: &AppState, files: &[PathBuf], subject: &str) -> Result<()> {
        let mut cmd = Command::new("xdg-email");
        cmd.arg("--subject").arg(subject);
        for f in files {
            cmd.arg("--attach").arg(f);
        }
        let status = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status()?;
        if !status.success() {
            bail!("xdg-email failed ({status})");
        }
        Ok(())
    }

    pub fn whatsapp_installed() -> bool {
        Command::new("xdg-mime")
            .args(["query", "default", "x-scheme-handler/whatsapp"])
            .output()
            .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false)
    }

    pub fn open_whatsapp_with(_files: &[PathBuf]) -> Result<bool> {
        tauri_plugin_opener::open_url("whatsapp://", None::<&str>).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_subjects() {
        assert_eq!(url_escape("2 photos & more"), "2%20photos%20%26%20more");
    }

    #[test]
    fn counts_items() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(count(&[p("a.jpg")]), "The photo");
        assert_eq!(count(&[p("a.mov")]), "The video");
        assert_eq!(count(&[p("a.jpg"), p("b.jpg")]), "2 photos");
        assert_eq!(count(&[p("a.jpg"), p("b.mp4")]), "2 items");
    }

    #[test]
    fn unique_names() {
        let d = tempfile::Builder::new().prefix("lumio-share-test-").tempdir().unwrap().keep();
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.jpg"), b"x").unwrap();
        assert_eq!(unique(&d, "a", "jpg"), d.join("a (2).jpg"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
