//! Lumio – a lightweight, open-source photo browser and digital asset manager.

mod ai;
mod commands;
mod db;
mod exiftool;
mod fileops;
mod histogram;
mod history;
mod indexer;
mod logfile;
mod metadata;
mod notify;
mod offline;
mod openwith;
mod pending;
mod rename;
mod share;
mod smugmug;
mod state;
mod thumbs;
mod videoserver;
mod watcher;
mod webexport;

use parking_lot::Mutex;
use state::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::http::{header, Response, StatusCode};
use tauri::Manager;

fn respond(status: StatusCode, mime: &str, body: Vec<u8>, cache: bool) -> Response<Vec<u8>> {
    let mut b = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    if cache {
        b = b.header(header::CACHE_CONTROL, "max-age=31536000, immutable");
    }
    b.body(body).unwrap()
}

fn not_found() -> Response<Vec<u8>> {
    respond(StatusCode::NOT_FOUND, "text/plain", b"not found".to_vec(), false)
}

fn id_from_uri(uri: &tauri::http::Uri) -> Option<i64> {
    uri.path().trim_matches('/').split('/').next()?.parse().ok()
}

fn mime_for(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "heic" | "heif" => "image/heic",
        _ => "application/octet-stream",
    }
}

/// How much of a video one request returns. The player asks for more as it plays,
/// so a long film on a network drive is never read into memory whole.
const VIDEO_CHUNK: u64 = 4 * 1024 * 1024;

/// Parse a `Range: bytes=start-end` header (single ranges only, which is what players send).
fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let (start, end) = if a.is_empty() {
        // "bytes=-500": the last 500 bytes.
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len.checked_sub(1)?)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() { len.checked_sub(1)? } else { b.parse::<u64>().ok()?.min(len.checked_sub(1)?) };
        (start, end)
    };
    (start <= end && start < len).then_some((start, end))
}

/// Serve a video in pieces (HTTP range requests), which is what the webviews' players
/// need to start quickly and to seek.
fn serve_video(path: &std::path::Path, ext: &str, range: Option<&str>) -> Response<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return not_found() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mime = metadata::video_mime(ext);
    let base = || {
        Response::builder()
            .header(header::CONTENT_TYPE, mime)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
    };
    let wanted = match range {
        Some(h) => match parse_range(h, len) {
            Some(r) => Some(r),
            None => {
                return base()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                    .body(vec![])
                    .unwrap()
            }
        },
        None => None,
    };
    // Small files without a range go back whole.
    if wanted.is_none() && len <= VIDEO_CHUNK {
        let mut body = Vec::with_capacity(len as usize);
        return match f.read_to_end(&mut body) {
            Ok(_) => base().status(StatusCode::OK).header(header::CONTENT_LENGTH, body.len()).body(body).unwrap(),
            Err(_) => not_found(),
        };
    }
    let (start, end) = wanted.unwrap_or((0, len.saturating_sub(1)));
    let end = end.min(start + VIDEO_CHUNK - 1);
    let mut body = vec![0u8; (end - start + 1) as usize];
    if f.seek(SeekFrom::Start(start)).and_then(|_| f.read_exact(&mut body)).is_err() {
        return not_found();
    }
    base()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
        .header(header::CONTENT_LENGTH, body.len())
        .body(body)
        .unwrap()
}

/// Serves `photo://localhost/<id>`: the original if the webview can show it, otherwise a
/// generated JPEG preview; when the drive is offline, the saved offline preview or,
/// failing that, the thumbnail. Videos are streamed in pieces (see `serve_video`).
fn serve_photo(state: &AppState, id: i64, range: Option<&str>) -> Response<Vec<u8>> {
    let (p, waiting_at) = {
        let conn = state.conn.lock();
        let Ok(Some(p)) = db::get_photo(&conn, id) else { return not_found() };
        (p, pending::source_of(&conn, id).ok().flatten())
    };
    // A photo whose move is still waiting is really still at its old place.
    let path = PathBuf::from(waiting_at.as_deref().unwrap_or(&p.path));
    if metadata::is_video(&p.ext) {
        return serve_video(&path, &p.ext, range);
    }
    if path.exists() {
        if metadata::browser_native(&p.ext) {
            if let Ok(bytes) = std::fs::read(&path) {
                return respond(StatusCode::OK, mime_for(&p.ext), bytes, false);
            }
        }
        let cached = state.previews_dir.join(format!("v2-{}-{}-{}.jpg", p.id, p.mtime, p.orientation.unwrap_or(1)));
        if let Ok(bytes) = std::fs::read(&cached) {
            return respond(StatusCode::OK, "image/jpeg", bytes, false);
        }
        if let Ok(bytes) = thumbs::make_preview(&path, &p.ext, p.orientation, &state.et, &state.tmp_dir) {
            let _ = std::fs::write(&cached, &bytes);
            return respond(StatusCode::OK, "image/jpeg", bytes, false);
        }
    }
    serve_saved(state, id, true)
}

/// The saved offline preview (when `large`) or the thumbnail, from Lumio's own folders,
/// so it works with the drive unplugged. Falls back to the thumbnail, uncached, so the
/// sharper preview shows once it has been made.
fn serve_saved(state: &AppState, id: i64, large: bool) -> Response<Vec<u8>> {
    if large {
        if let Ok(b) = std::fs::read(thumbs::offline_preview_path(&state.offline_dir, id)) {
            return respond(StatusCode::OK, "image/jpeg", b, false);
        }
    }
    match std::fs::read(thumbs::thumb_path(&state.thumbs_dir, id)) {
        Ok(b) => respond(StatusCode::OK, "image/jpeg", b, !large),
        Err(_) => not_found(),
    }
}

/// Where the player should load videos from, when that isn't `photo://` (see `videoserver`).
struct VideoBase(Option<String>);

#[tauri::command]
fn video_base(base: tauri::State<'_, VideoBase>) -> Option<String> {
    base.0.clone()
}

fn clean_previews(dir: &std::path::Path) {
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(14 * 24 * 3600);
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.metadata().and_then(|m| m.modified()).map(|t| t < cutoff).unwrap_or(false) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol("thumb", |ctx, request, responder| {
            let state = ctx.app_handle().state::<Arc<AppState>>().inner().clone();
            let id = id_from_uri(request.uri());
            // `?s=L` asks for the sharper offline preview (for the large view and filmstrip).
            let large = request.uri().query().is_some_and(|q| q.split('&').any(|kv| kv == "s=L"));
            std::thread::spawn(move || {
                responder.respond(id.map(|id| serve_saved(&state, id, large)).unwrap_or_else(not_found));
            });
        })
        .register_asynchronous_uri_scheme_protocol("photo", |ctx, request, responder| {
            let state = ctx.app_handle().state::<Arc<AppState>>().inner().clone();
            let id = id_from_uri(request.uri());
            let range = request.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::to_owned);
            std::thread::spawn(move || {
                responder.respond(id.map(|id| serve_photo(&state, id, range.as_deref())).unwrap_or_else(not_found));
            });
        })
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let thumbs_dir = data.join("thumbnails");
            let offline_dir = data.join("previews");
            let previews_dir = app.path().app_cache_dir()?.join("previews");
            let tmp_dir = app.path().app_cache_dir()?.join("tmp");
            for d in [&data, &thumbs_dir, &offline_dir, &previews_dir, &tmp_dir] {
                std::fs::create_dir_all(d)?;
            }
            logfile::init(data.join("lumio.log"));
            log::info!("Lumio {} starting", app.package_info().version);
            clean_previews(&previews_dir);
            let db_path = data.join("catalogue.sqlite");
            let conn = db::open(&db_path)?;
            db::migrate(&conn)?;
            let et = Arc::new(exiftool::ExifTool::new(app.path().resource_dir().ok()));
            if !et.available() {
                log::warn!("ExifTool not found – metadata will be limited");
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let state = Arc::new(AppState {
                app: app.handle().clone(),
                db_path,
                conn: Mutex::new(conn),
                thumbs_dir,
                previews_dir,
                offline_dir,
                tmp_dir,
                et,
                jobs: Mutex::new(tx),
                watcher: Mutex::new(None),
                chat: Mutex::new(Default::default()),
            });
            app.manage(state.clone());
            // WebKitGTK won't play videos from photo://, so Linux streams them over local HTTP.
            let video_base = if cfg!(target_os = "linux") {
                match videoserver::start(state.clone()) {
                    Ok(b) => Some(b),
                    Err(e) => {
                        log::warn!("video server could not start: {e}");
                        None
                    }
                }
            } else {
                None
            };
            app.manage(VideoBase(video_base));
            {
                let st = state.clone();
                std::thread::Builder::new().name("indexer".into()).spawn(move || indexer::run_worker(st, rx))?;
            }
            // Catch up on changes made while Lumio was closed.
            for r in db::list_roots(&state.conn.lock())? {
                if r.online {
                    state.queue(indexer::Job::ScanRoot(r.id));
                }
            }
            state.queue(indexer::Job::ApplyPending);
            state.queue(indexer::Job::Previews);
            // Drives come back without telling us: once a minute, if moves are waiting,
            // let the worker see whether it can do them now.
            {
                let st = Arc::downgrade(&state);
                std::thread::Builder::new().name("pending-moves".into()).spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    let Some(st) = st.upgrade() else { break };
                    if pending::count(&st.conn.lock()) > 0 {
                        st.queue(indexer::Job::ApplyPending);
                    }
                })?;
            }
            watcher::restart(&state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            video_base,
            commands::get_roots,
            commands::add_root,
            commands::remove_root,
            commands::rescan,
            commands::refresh_photos,
            commands::folder_children,
            commands::create_folder,
            commands::rename_folder,
            commands::delete_folder,
            commands::move_folder,
            commands::query_photos,
            commands::get_photo,
            commands::get_photos,
            commands::move_photos,
            commands::pending_moves,
            commands::cancel_pending_moves,
            commands::retry_pending_moves,
            commands::history_list,
            commands::history_undo,
            commands::histogram,
            commands::offline_preview_stats,
            commands::clear_offline_previews,
            commands::delete_photos,
            commands::rename_photo,
            commands::rename_preview,
            commands::rename_photos,
            commands::update_metadata,
            commands::rotate_photos,
            commands::save_video_thumb,
            commands::stack_photos,
            commands::unstack_photos,
            commands::set_stack_top,
            commands::list_open_with,
            commands::open_with,
            commands::get_settings,
            commands::set_setting,
            commands::app_status,
            commands::ai_settings,
            commands::ai_set_key,
            commands::ai_chat,
            commands::ai_reset,
            commands::geocode,
            commands::reveal,
            commands::open_default,
            commands::share_options,
            commands::share_sheet,
            commands::share_email,
            commands::share_whatsapp,
            commands::share_site,
            commands::share_copy,
            commands::share_show,
            smugmug::cmd::smugmug_status,
            smugmug::cmd::smugmug_set_app_key,
            smugmug::cmd::smugmug_begin_sign_in,
            smugmug::cmd::smugmug_finish_sign_in,
            smugmug::cmd::smugmug_sign_out,
            smugmug::cmd::smugmug_albums,
            smugmug::cmd::smugmug_create_album,
            smugmug::cmd::smugmug_upload,
            smugmug::cmd::smugmug_cancel,
            smugmug::cmd::smugmug_open,
            webexport::cmd::web_export,
            webexport::cmd::web_export_cancel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Lumio");
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=0-1", 1000), Some((0, 1)));
        assert_eq!(parse_range("bytes=500-2000", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=5-2", 1000), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
    }
}
