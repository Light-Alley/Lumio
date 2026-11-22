//! "Export web-ready copies": resized JPEGs for a website such as Squarespace.
//!
//! The originals are never touched. Each copy is decoded the same way the viewer does it
//! (so RAW and HEIC work), turned upright, shrunk to the chosen long edge, saved as an
//! sRGB-style 8-bit JPEG, and given the original's metadata back via ExifTool (minus the
//! location, unless the user wants to keep it). SmugMug uploads reuse [`render_jpeg`]
//! for files SmugMug can't take as they are.

use crate::db::{self, Photo};
use crate::exiftool;
use crate::indexer;
use crate::metadata;
use crate::state::AppState;
use crate::thumbs;
use anyhow::{anyhow, Result};
use image::imageops::FilterType;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tauri::Emitter;

#[derive(Deserialize, Clone, Debug)]
pub struct ExportOptions {
    /// Longest side in pixels; `None` keeps the full size.
    pub long_edge: Option<u32>,
    /// JPEG quality, 50 to 100.
    pub quality: u8,
    /// Leave GPS coordinates out of the copies (recommended for public websites).
    pub strip_location: bool,
}

#[derive(Serialize, Default)]
pub struct ExportReport {
    pub done: usize,
    pub failed: Vec<(String, String)>,
    pub skipped_videos: usize,
    pub cancelled: bool,
    pub folder: String,
}

static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}

/// Decode `p`, resize it and write a JPEG to `dest`, copying metadata across.
pub fn render_jpeg(state: &AppState, p: &Photo, opts: &ExportOptions, dest: &Path) -> Result<()> {
    let src = Path::new(&p.path);
    if !src.exists() {
        return Err(anyhow!("not available (is its drive connected?)"));
    }
    // For RAW files `want` decides whether the embedded preview is big enough; 0 means
    // "take the largest embedded preview", which for modern cameras is full size.
    let want = opts.long_edge.unwrap_or(0);
    let mut img = thumbs::load_image(src, &p.ext, p.orientation, want, &state.et, &state.tmp_dir)?;
    if let Some(edge) = opts.long_edge {
        if img.width().max(img.height()) > edge {
            img = img.resize(edge, edge, FilterType::Lanczos3);
        }
    }
    let bytes = thumbs::encode_jpeg(&img, opts.quality.clamp(50, 100))?;
    std::fs::write(dest, bytes)?;
    if state.et.available() {
        // The pixels are already upright, so the orientation tag and any embedded
        // thumbnails/previews (which would be the wrong size or way round) stay behind.
        // -TagsFromFile expands '%' codes, so a source with '%' in its path goes through a
        // stand-in (see ExifTool::plain).
        let source = state.et.plain(src)?;
        let mut args: Vec<String> = ["-overwrite_original", "-m", "-codedcharacterset=utf8", "-TagsFromFile"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.push(exiftool::fmt_path_arg(source.path())?);
        for a in ["-all:all", "--Orientation", "--ThumbnailImage", "--PreviewImage", "--JpgFromRaw", "--MakerNotes:all"]
        {
            args.push(a.into());
        }
        // Titles, captions and keywords edited in Lumio may only be in an XMP sidecar.
        let sidecar = indexer::find_sidecar(src).map(|x| state.et.plain(&x)).transpose()?;
        if let Some(xmp) = &sidecar {
            args.push("-TagsFromFile".into());
            args.push(exiftool::fmt_path_arg(xmp.path())?);
            args.push("-XMP:all".into());
        }
        args.push(exiftool::path_arg(dest)?);
        if let Err(e) = state.et.run(&args) {
            log::info!("could not copy metadata to {}: {e:#}", dest.display());
        }
        // Excluding GPS while copying doesn't catch every tag, so remove it afterwards:
        // coordinates and the place names (city, region, country, sub-location).
        if opts.strip_location {
            state.et.write(dest, &metadata::strip_location_args())?;
        }
    }
    Ok(())
}

/// A file name in `dir` based on `stem` that isn't taken yet.
fn free_name(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.jpg"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| dir.join(format!("{stem}-{n}.jpg"))).find(|p| !p.exists()).unwrap()
}

pub fn export(state: &AppState, ids: &[i64], folder: &Path, opts: &ExportOptions) -> Result<ExportReport> {
    CANCEL.store(false, Ordering::Relaxed);
    if !folder.is_absolute() || folder.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(anyhow!("That folder is not usable."));
    }
    if !folder.is_dir() {
        return Err(anyhow!("That folder is not available."));
    }
    let photos = db::get_photos(&state.conn.lock(), ids)?;
    let (videos, photos): (Vec<Photo>, Vec<Photo>) = photos.into_iter().partition(|p| metadata::is_video(&p.ext));
    let total = photos.len();
    let done = AtomicUsize::new(0);
    let mut report = ExportReport {
        skipped_videos: videos.len(),
        folder: folder.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let emit = |n: usize, name: &str| {
        let _ =
            state.app.emit("web-export-progress", serde_json::json!({ "done": n, "total": total, "current": name }));
    };
    emit(0, "");
    // One at a time keeps memory low for large RAW files and gives unique names safely.
    for p in &photos {
        if CANCEL.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let stem = Path::new(&p.filename)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("photo-{}", p.id));
        let dest = free_name(folder, &stem);
        match render_jpeg(state, p, opts, &dest) {
            Ok(()) => report.done += 1,
            Err(e) => {
                let _ = std::fs::remove_file(&dest);
                report.failed.push((p.filename.clone(), format!("{e:#}")));
            }
        }
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        emit(n, &p.filename);
    }
    Ok(report)
}

/// The commands the user interface calls.
pub mod cmd {
    use super::*;
    use std::sync::Arc;
    use tauri::State;

    #[tauri::command]
    pub async fn web_export(
        state: State<'_, Arc<AppState>>,
        ids: Vec<i64>,
        folder: String,
        opts: ExportOptions,
    ) -> Result<ExportReport, String> {
        let st = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || export(&st, &ids, Path::new(&folder), &opts))
            .await
            .map_err(|x| x.to_string())?
            .map_err(|x| format!("{x:#}"))
    }

    #[tauri::command]
    pub fn web_export_cancel() {
        cancel();
    }
}
