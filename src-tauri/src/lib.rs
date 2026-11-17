//! Lumio – a lightweight, open-source photo browser and digital asset manager.

mod ai;
mod commands;
mod db;
mod exiftool;
mod fileops;
mod indexer;
mod logfile;
mod metadata;
mod notify;
mod openwith;
mod state;
mod thumbs;
mod watcher;

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

/// Serves `photo://localhost/<id>`: the original if the webview can show it, otherwise a
/// generated JPEG preview; when the drive is offline, the cached thumbnail.
fn serve_photo(state: &AppState, id: i64) -> Response<Vec<u8>> {
    let Ok(Some(p)) = db::get_photo(&state.conn.lock(), id) else { return not_found() };
    let path = PathBuf::from(&p.path);
    if path.exists() {
        if metadata::browser_native(&p.ext) {
            if let Ok(bytes) = std::fs::read(&path) {
                return respond(StatusCode::OK, mime_for(&p.ext), bytes, false);
            }
        }
        let cached = state.previews_dir.join(format!("{}-{}-{}.jpg", p.id, p.mtime, p.orientation.unwrap_or(1)));
        if let Ok(bytes) = std::fs::read(&cached) {
            return respond(StatusCode::OK, "image/jpeg", bytes, false);
        }
        if let Ok(bytes) = thumbs::make_preview(&path, &p.ext, p.orientation, &state.et, &state.tmp_dir) {
            let _ = std::fs::write(&cached, &bytes);
            return respond(StatusCode::OK, "image/jpeg", bytes, false);
        }
    }
    match std::fs::read(thumbs::thumb_path(&state.thumbs_dir, id)) {
        Ok(b) => respond(StatusCode::OK, "image/jpeg", b, false),
        Err(_) => not_found(),
    }
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
            std::thread::spawn(move || {
                let resp = match id.and_then(|id| std::fs::read(thumbs::thumb_path(&state.thumbs_dir, id)).ok()) {
                    Some(b) => respond(StatusCode::OK, "image/jpeg", b, true),
                    None => not_found(),
                };
                responder.respond(resp);
            });
        })
        .register_asynchronous_uri_scheme_protocol("photo", |ctx, request, responder| {
            let state = ctx.app_handle().state::<Arc<AppState>>().inner().clone();
            let id = id_from_uri(request.uri());
            std::thread::spawn(move || {
                responder.respond(id.map(|id| serve_photo(&state, id)).unwrap_or_else(not_found));
            });
        })
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let thumbs_dir = data.join("thumbnails");
            let previews_dir = app.path().app_cache_dir()?.join("previews");
            let tmp_dir = app.path().app_cache_dir()?.join("tmp");
            for d in [&data, &thumbs_dir, &previews_dir, &tmp_dir] {
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
                tmp_dir,
                et,
                jobs: Mutex::new(tx),
                watcher: Mutex::new(None),
                chat: Mutex::new(Default::default()),
            });
            app.manage(state.clone());
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
            watcher::restart(&state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            commands::delete_photos,
            commands::rename_photo,
            commands::update_metadata,
            commands::rotate_photos,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Lumio");
}
