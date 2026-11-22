//! Telling people when a newer Lumio is out, and (when the release was signed for it)
//! installing it from inside the app.
//!
//! Two parts, so that it works before and after the updater keys exist:
//!
//! - **Is there a newer version?** asks GitHub for the latest published release. This
//!   needs no keys and works for every install (drafts and pre-releases are ignored).
//! - **Install and restart** uses Tauri's updater, which only accepts a download signed
//!   with Lumio's updater key. The release workflow switches it on when that key is set
//!   (see docs/updates.md); until then, and on Linux .deb/.rpm installs, the notice
//!   offers the download page instead.
//!
//! Nothing about the user or their photos is sent: a plain HTTPS request to GitHub.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const REPO_URL: &str = "https://github.com/Light-Alley/Lumio";
const LATEST_API: &str = "https://api.github.com/repos/Light-Alley/Lumio/releases/latest";

/// Whether the updater plugin was set up (the build carries an updater public key).
pub struct Installer(pub bool);

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    /// The latest release is newer than this copy.
    pub newer: bool,
    pub name: String,
    pub notes: String,
    /// The release page, for downloading by hand.
    pub url: String,
    pub published: String,
    /// "Install and restart" can be offered.
    pub can_install: bool,
}

/// "v1.2.0", "1.2.0" or "Lumio v1.2.0" -> 1.2.0.
fn parse_version(tag: &str) -> Option<semver::Version> {
    let t = tag.trim();
    let t = t.rsplit(' ').next().unwrap_or(t);
    let t = t.trim_start_matches(['v', 'V']);
    semver::Version::parse(t).ok()
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// A page of Lumio's own GitHub project: https, github.com, and the Light-Alley/Lumio
/// repository (GitHub treats owner and repository names without regard to case).
/// Parsed rather than compared as text, so "github.com.evil.example", user names in the
/// address, or "/Light-Alley/Lumio-fake" don't pass.
fn is_project_page(u: &str) -> bool {
    let Ok(url) = url::Url::parse(u) else { return false };
    if url.scheme() != "https" || url.host_str() != Some("github.com") || url.port().is_some() {
        return false;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let mut parts = url.path_segments().into_iter().flatten();
    let (owner, repo) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    owner.eq_ignore_ascii_case("Light-Alley") && repo.eq_ignore_ascii_case("Lumio")
}

/// Read GitHub's answer for the latest release.
fn from_release(v: &serde_json::Value, current: &str, can_install: bool) -> Option<UpdateInfo> {
    let tag = v["tag_name"].as_str()?;
    let latest = parse_version(tag)?.to_string();
    let newer = is_newer(&latest, current);
    Some(UpdateInfo {
        current: current.to_string(),
        latest,
        newer,
        name: v["name"].as_str().unwrap_or(tag).to_string(),
        notes: v["body"].as_str().unwrap_or_default().chars().take(6000).collect(),
        url: v["html_url"]
            .as_str()
            .filter(|u| is_project_page(u))
            .map(str::to_string)
            .unwrap_or_else(|| format!("{REPO_URL}/releases/latest")),
        published: v["published_at"].as_str().unwrap_or_default().to_string(),
        can_install: newer && can_install,
    })
}

/// In-app install works where the updater can replace the app: macOS, Windows, and the
/// Linux AppImage (a .deb or .rpm install is updated by downloading the new package).
fn install_supported(app: &AppHandle) -> bool {
    let on = app.try_state::<Installer>().is_some_and(|s| s.0);
    on && (!cfg!(target_os = "linux") || std::env::var_os("APPIMAGE").is_some())
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(format!("Lumio/{current}"))
        .build()
        .map_err(|e| e.to_string())?;
    // Development builds can point at a test server: LUMIO_UPDATE_API=http://127.0.0.1:8000/latest.json
    let api = if cfg!(debug_assertions) {
        std::env::var("LUMIO_UPDATE_API").unwrap_or_else(|_| LATEST_API.into())
    } else {
        LATEST_API.into()
    };
    let resp = client
        .get(api)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Could not reach GitHub to check for updates. Are you online? ({e})"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err("No published Lumio release was found on GitHub yet.".into());
    }
    if !status.is_success() {
        return Err(format!(
            "GitHub didn't answer the update check (HTTP {}). Try again later.",
            status.as_u16()
        ));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    from_release(&v, &current, install_supported(&app)).ok_or_else(|| "GitHub's answer had no version number.".into())
}

#[derive(Serialize, Clone)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

/// Download the update, check its signature, install it and restart Lumio.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    if !install_supported(&app) {
        return Err("This copy of Lumio can't update itself. Download the new version from the release page.".into());
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("Could not get the update: {e}"))?
        .ok_or("The update isn't ready to install yet (its files may still be uploading). Try again in a few minutes, or download it from the release page.")?;
    let mut downloaded = 0u64;
    let a = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = a.emit("update-progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| format!("The update could not be installed: {e}"))?;
    log::info!("update {} installed, restarting", update.version);
    app.restart();
}

#[tauri::command]
pub fn open_project_page(url: Option<String>) -> Result<(), String> {
    // Only ever open Lumio's own GitHub pages from here (release notes, guide, issues).
    let url = url
        .filter(|u| is_project_page(u))
        .unwrap_or_else(|| format!("{REPO_URL}/releases/latest"));
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_lumios_own_pages_are_opened() {
        assert!(is_project_page("https://github.com/Light-Alley/Lumio/releases/tag/v2.0.0"));
        assert!(is_project_page("https://github.com/LIght-Alley/Lumio/issues"));
        assert!(is_project_page("https://github.com/light-alley/lumio"));
        assert!(!is_project_page("https://github.com/Light-Alley/Lumio-fake/releases"));
        assert!(!is_project_page("https://github.com.evil.example/Light-Alley/Lumio"));
        assert!(!is_project_page("https://user@github.com/Light-Alley/Lumio"));
        assert!(!is_project_page("http://github.com/Light-Alley/Lumio"));
        assert!(!is_project_page("https://github.com:8443/Light-Alley/Lumio"));
        assert!(!is_project_page("https://github.com/Someone/Lumio"));
        assert!(!is_project_page("javascript:alert(1)"));
        let bad = json!({"tag_name": "v9.0.0", "html_url": "https://evil.example/download"});
        assert_eq!(from_release(&bad, "1.0.0", false).unwrap().url, format!("{REPO_URL}/releases/latest"));
    }

    #[test]
    fn versions() {
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(is_newer("1.1.0", "1.0.9"));
        assert!(is_newer("Lumio v2.0.0", "1.9.9"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("v0.3.8", "1.0.0"));
        assert!(!is_newer("nightly", "1.0.0"));
        // A pre-release of the same version is older than the release.
        assert!(!is_newer("1.0.0-beta.1", "1.0.0"));
        assert!(is_newer("1.0.0", "1.0.0-beta.1"));
    }

    #[test]
    fn reads_a_release() {
        let r = json!({
            "tag_name": "v1.1.0", "name": "Lumio v1.1.0", "body": "New: menu bar",
            "html_url": "https://github.com/Light-Alley/Lumio/releases/tag/v1.1.0",
            "published_at": "2026-12-01T10:00:00Z"
        });
        let u = from_release(&r, "1.0.0", true).unwrap();
        assert!(u.newer && u.can_install);
        assert_eq!(u.latest, "1.1.0");
        assert_eq!(u.notes, "New: menu bar");
        let same = from_release(&r, "1.1.0", true).unwrap();
        assert!(!same.newer && !same.can_install);
        assert!(!from_release(&r, "1.0.0", false).unwrap().can_install);
        assert!(from_release(&json!({"name": "x"}), "1.0.0", true).is_none());
    }
}
