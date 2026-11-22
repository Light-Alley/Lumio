//! "Open with…" – lists the applications the operating system says can open a file.

use anyhow::{bail, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    /// Display name, e.g. "Adobe Photoshop 2025".
    pub name: String,
    /// What we pass back to `open_with` (an app bundle path, exe path or desktop file).
    pub id: String,
    pub is_default: bool,
}

pub fn list(file: &Path) -> Vec<AppInfo> {
    let mut v = imp::list(file).unwrap_or_default();
    v.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let mut seen = std::collections::HashSet::new();
    v.retain(|a| seen.insert(a.name.to_lowercase()));
    v
}

/// Opens `file` with `app_id`, which must be one of the applications the operating system
/// itself offers for that file (see `list`). The id comes from the user interface, so it
/// is never trusted on its own: anything else is refused, whatever it points at.
pub fn open_with(file: &Path, app_id: &str) -> Result<()> {
    if !file.exists() {
        bail!("The file is not available (is its drive connected?)");
    }
    if !list(file).iter().any(|a| a.id == app_id) {
        bail!("That application is not offered for this file.");
    }
    imp::open_with(file, app_id)
}

/// Opens `file` with an application the person picked themselves in the system file
/// dialog (shown by Rust, so the path never comes from the user interface).
pub fn open_with_chosen(file: &Path, chosen: &Path) -> Result<()> {
    if !file.exists() {
        bail!("The file is not available (is its drive connected?)");
    }
    if !chosen_app_ok(chosen) {
        bail!("That is not an application.");
    }
    imp::open_with(file, &chosen.to_string_lossy())
}

/// What may be picked as "another application": an .app bundle on macOS, an .exe on
/// Windows, an installed .desktop entry or an executable file on Linux.
fn chosen_app_ok(p: &Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if cfg!(target_os = "macos") {
        ext == "app" && p.is_dir()
    } else if cfg!(target_os = "windows") {
        ext == "exe" && p.is_file()
    } else {
        imp::chosen_ok(p)
    }
}

// ---------------------------------------------------------------- macOS
#[cfg(target_os = "macos")]
mod imp {
    use super::AppInfo;
    use anyhow::Result;
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::url::{CFURL, CFURLRef};
    use std::path::{Path, PathBuf};

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn LSCopyApplicationURLsForURL(url: CFURLRef, roles: u32) -> CFArrayRef;
        fn LSCopyDefaultApplicationURLForURL(url: CFURLRef, roles: u32, err: *mut *const std::ffi::c_void) -> CFURLRef;
    }
    const ROLES_ALL: u32 = 0xFFFF_FFFF;

    fn app_name(p: &Path) -> String {
        p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
    }

    pub fn list(file: &Path) -> Result<Vec<AppInfo>> {
        let Some(url) = CFURL::from_path(file, false) else { return Ok(vec![]) };
        let mut out = vec![];
        let default: Option<PathBuf> = unsafe {
            let r = LSCopyDefaultApplicationURLForURL(url.as_concrete_TypeRef(), ROLES_ALL, std::ptr::null_mut());
            if r.is_null() { None } else { CFURL::wrap_under_create_rule(r).to_path() }
        };
        unsafe {
            let arr = LSCopyApplicationURLsForURL(url.as_concrete_TypeRef(), ROLES_ALL);
            if !arr.is_null() {
                let arr: CFArray<CFURL> = CFArray::wrap_under_create_rule(arr);
                for u in arr.iter() {
                    if let Some(p) = u.to_path() {
                        out.push(AppInfo {
                            name: app_name(&p),
                            is_default: default.as_ref() == Some(&p),
                            id: p.to_string_lossy().to_string(),
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn open_with(file: &Path, app: &str) -> Result<()> {
        // `--` so a file name can never be read as an option of `open`.
        std::process::Command::new("/usr/bin/open").arg("-a").arg(app).arg("--").arg(file).spawn()?;
        Ok(())
    }
}

// ---------------------------------------------------------------- Windows
#[cfg(target_os = "windows")]
mod imp {
    use super::AppInfo;
    use anyhow::{anyhow, Result};
    use std::path::Path;
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, IDataObject, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        BHID_DataObject, IAssocHandler, IShellItem, SHAssocEnumHandlers, SHCreateItemFromParsingName,
        ASSOC_FILTER_RECOMMENDED,
    };

    fn handlers(file: &Path) -> Result<Vec<IAssocHandler>> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let ext = format!(".{}", file.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default());
            let en = SHAssocEnumHandlers(&HSTRING::from(ext), ASSOC_FILTER_RECOMMENDED)?;
            let mut out = vec![];
            loop {
                let mut buf: [Option<IAssocHandler>; 1] = [None];
                let mut fetched = 0u32;
                if en.Next(&mut buf, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                if let Some(h) = buf[0].take() {
                    out.push(h);
                }
            }
            Ok(out)
        }
    }

    unsafe fn take_pwstr(p: windows::core::PWSTR) -> String {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }

    pub fn list(file: &Path) -> Result<Vec<AppInfo>> {
        let mut out = vec![];
        for h in handlers(file)? {
            unsafe {
                let name = h.GetUIName().map(|p| take_pwstr(p)).unwrap_or_default();
                let id = h.GetName().map(|p| take_pwstr(p)).unwrap_or_default();
                if !name.is_empty() && !id.is_empty() {
                    out.push(AppInfo { name, id, is_default: false });
                }
            }
        }
        if let Some(first) = out.first_mut() {
            first.is_default = true; // Windows lists the default handler first.
        }
        Ok(out)
    }

    pub fn open_with(file: &Path, app: &str) -> Result<()> {
        // Prefer invoking through the shell (works for Store apps such as Photos).
        for h in handlers(file)? {
            unsafe {
                let id = h.GetName().map(|p| take_pwstr(p)).unwrap_or_default();
                if id == app {
                    let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(file.as_os_str()), None)?;
                    let data: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
                    h.Invoke(&data)?;
                    return Ok(());
                }
            }
        }
        // Otherwise it is an .exe the person picked in the file dialog (see `open_with_chosen`).
        std::process::Command::new(app).arg(file).spawn().map_err(|e| anyhow!("could not start {app}: {e}"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------- Linux / BSD
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use super::AppInfo;
    use anyhow::{anyhow, Result};
    use std::path::{Path, PathBuf};

    fn mime_for(file: &Path) -> &'static str {
        match file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "jpe" => "image/jpeg",
            "png" => "image/png",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "tif" | "tiff" => "image/tiff",
            "heic" | "heif" => "image/heif",
            "avif" => "image/avif",
            "dng" => "image/x-adobe-dng",
            "cr2" => "image/x-canon-cr2",
            "cr3" => "image/x-canon-cr3",
            "nef" => "image/x-nikon-nef",
            "arw" => "image/x-sony-arw",
            "raf" => "image/x-fuji-raf",
            "orf" => "image/x-olympus-orf",
            "rw2" => "image/x-panasonic-rw2",
            "mp4" | "m4v" => "video/mp4",
            "mov" => "video/quicktime",
            "avi" => "video/x-msvideo",
            "mkv" => "video/x-matroska",
            "webm" => "video/webm",
            "mts" | "m2ts" => "video/mp2t",
            "3gp" => "video/3gpp",
            "wmv" => "video/x-ms-wmv",
            _ if crate::metadata::is_video(&file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()) => "video/*",
            _ => "image/x-raw",
        }
    }

    /// The broad family ("image/*" or "video/*") an application may declare instead.
    fn family(mime: &str) -> &'static str {
        if mime.starts_with("video/") {
            "video/*"
        } else {
            "image/*"
        }
    }

    /// True when `p` is a .desktop entry inside one of the system's application folders.
    fn trusted_desktop(p: &Path) -> bool {
        let Ok(canon) = std::fs::canonicalize(p) else { return false };
        app_dirs().iter().filter_map(|d| std::fs::canonicalize(d).ok()).any(|d| canon.parent() == Some(d.as_path()))
    }

    pub fn chosen_ok(p: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        if p.extension().and_then(|e| e.to_str()) == Some("desktop") {
            return trusted_desktop(p);
        }
        std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }

    fn app_dirs() -> Vec<PathBuf> {
        let mut dirs = vec![];
        let home = std::env::var("HOME").unwrap_or_default();
        let data_home = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{home}/.local/share"));
        dirs.push(PathBuf::from(&data_home).join("applications"));
        let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        for d in data_dirs.split(':') {
            dirs.push(PathBuf::from(d).join("applications"));
        }
        dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
        dirs.push(PathBuf::from(format!("{home}/.local/share/flatpak/exports/share/applications")));
        dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));
        dirs
    }

    struct Desktop {
        name: String,
        exec: String,
        mimes: Vec<String>,
    }

    fn parse(path: &Path) -> Option<Desktop> {
        let text = std::fs::read_to_string(path).ok()?;
        let (mut name, mut exec, mut mimes, mut in_entry, mut hidden) = (None, None, vec![], false, false);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
                continue;
            }
            if !in_entry {
                continue;
            }
            if let Some(v) = line.strip_prefix("Name=") { name.get_or_insert(v.to_string()); }
            if let Some(v) = line.strip_prefix("Exec=") { exec = Some(v.to_string()); }
            if let Some(v) = line.strip_prefix("MimeType=") {
                mimes = v.split(';').filter(|s| !s.is_empty()).map(String::from).collect();
            }
            if line == "NoDisplay=true" || line == "Hidden=true" { hidden = true; }
        }
        if hidden { return None; }
        Some(Desktop { name: name?, exec: exec?, mimes })
    }

    pub fn list(file: &Path) -> Result<Vec<AppInfo>> {
        let mime = mime_for(file);
        let mut out = vec![];
        let mut seen = std::collections::HashSet::new();
        for dir in app_dirs() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("desktop") { continue; }
                let fname = p.file_name().unwrap().to_string_lossy().to_string();
                if !seen.insert(fname) { continue; }
                if let Some(d) = parse(&p) {
                    if d.mimes.iter().any(|m| m == mime || m == family(mime)) {
                        out.push(AppInfo { name: d.name, id: p.to_string_lossy().to_string(), is_default: false });
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn open_with(file: &Path, app: &str) -> Result<()> {
        let f = file.to_string_lossy().to_string();
        if app.ends_with(".desktop") {
            if !trusted_desktop(Path::new(app)) {
                return Err(anyhow!("that application entry is not installed on this computer"));
            }
            let d = parse(Path::new(app)).ok_or_else(|| anyhow!("cannot read {app}"))?;
            let words = shell_words::split(&d.exec)?;
            let mut args: Vec<String> = vec![];
            let mut used = false;
            for w in words {
                match w.as_str() {
                    "%f" | "%F" | "%u" | "%U" => { args.push(f.clone()); used = true; }
                    "%i" | "%c" | "%k" => {}
                    _ => args.push(w.replace("%%", "%")),
                }
            }
            if !used { args.push(f); }
            let (prog, rest) = args.split_first().ok_or_else(|| anyhow!("empty Exec line"))?;
            std::process::Command::new(prog).args(rest).spawn()?;
        } else {
            std::process::Command::new(app).arg(&f).spawn()?;
        }
        Ok(())
    }
}
