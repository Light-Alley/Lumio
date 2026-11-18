//! Thumbnail and preview generation. Thumbnails live in the app's data folder, so
//! photos on external or network drives can still be browsed when the drive is away.

use crate::exiftool::ExifTool;
use crate::metadata;
use anyhow::{anyhow, Result};
use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::io::Cursor;
use std::path::{Path, PathBuf};

pub const THUMB_SIZE: u32 = 480;
pub const PREVIEW_SIZE: u32 = 2560;

pub fn thumb_path(dir: &Path, id: i64) -> PathBuf {
    dir.join(format!("{:02x}", id % 256)).join(format!("{id}.jpg"))
}

fn decode_with_orientation(reader: ImageReader<impl std::io::BufRead + std::io::Seek>) -> Result<DynamicImage> {
    let mut decoder = reader.with_guessed_format()?.into_decoder()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok(img)
}

fn orientation_from_exif(o: Option<i64>) -> Orientation {
    o.and_then(|v| Orientation::from_exif(v as u8)).unwrap_or(Orientation::NoTransforms)
}

/// Load an image for display, decoding it ourselves or via an embedded preview.
/// `want` is the long side the caller needs; RAW files use their embedded JPEG when
/// it is at least that big, which is far quicker than a full RAW decode (and on a
/// network drive reads much less of the file).
pub fn load_image(path: &Path, ext: &str, orientation: Option<i64>, want: u32, et: &ExifTool, tmp: &Path) -> Result<DynamicImage> {
    let raw = metadata::is_raw(ext);
    let e = ext.to_ascii_lowercase();
    if !raw && !matches!(e.as_str(), "heic" | "heif" | "avif") {
        if let Ok(img) = ImageReader::open(path).map_err(anyhow::Error::from).and_then(decode_with_orientation) {
            return Ok(img);
        }
    }
    let embedded = || -> Result<Option<DynamicImage>> {
        if !et.available() {
            return Ok(None);
        }
        let Some(bytes) = et.extract_preview(path, tmp)? else { return Ok(None) };
        let mut img = decode_with_orientation(ImageReader::new(Cursor::new(bytes)))?;
        // Embedded previews are usually stored unrotated.
        img.apply_orientation(orientation_from_exif(orientation));
        Ok(Some(img))
    };
    let mut small: Option<DynamicImage> = None;
    if raw {
        match embedded() {
            Ok(Some(img)) if img.width().max(img.height()) >= want => return Ok(img),
            Ok(img) => small = img,
            Err(e) => log::info!("no embedded preview in {}: {e:#}", path.display()),
        }
    }
    #[cfg(target_os = "macos")]
    if matches!(e.as_str(), "heic" | "heif" | "avif") || raw {
        if let Ok(img) = sips_decode(path, tmp, orientation) {
            return Ok(img);
        }
    }
    if let Some(img) = small {
        return Ok(img);
    }
    if !raw {
        if let Some(img) = embedded()? {
            return Ok(img);
        }
    }
    Err(anyhow!("cannot decode {}", path.display()))
}

/// Decode with macOS's own image code (HEIC, AVIF and RAW). sips leaves the pixels as
/// they are stored and only carries the orientation tag over, so we turn the image here:
/// by the catalogue's orientation when we have one (it includes rotations saved in an
/// XMP sidecar), otherwise by the tag sips kept.
#[cfg(target_os = "macos")]
fn sips_decode(path: &Path, tmp: &Path, orientation: Option<i64>) -> Result<DynamicImage> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let out = tmp.join(format!("sips{}-{}.jpg", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let status = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg", "-Z", &PREVIEW_SIZE.to_string()])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(anyhow!("sips failed"));
    }
    let img = (|| -> Result<DynamicImage> {
        let mut decoder = ImageReader::open(&out)?.with_guessed_format()?.into_decoder()?;
        let kept = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        let mut img = DynamicImage::from_decoder(decoder)?;
        let turn = match orientation.filter(|o| (1..=8).contains(o)) {
            Some(o) => orientation_from_exif(Some(o)),
            None => kept,
        };
        img.apply_orientation(turn);
        Ok(img)
    })();
    let _ = std::fs::remove_file(&out);
    img
}

pub fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let rgb = img.to_rgb8();
    let mut buf = Vec::new();
    JpegEncoder::new_with_quality(&mut buf, quality).encode_image(&rgb)?;
    Ok(buf)
}

/// Create a thumbnail file. Returns (width, height) of the source image.
pub fn make_thumb(
    src: &Path,
    ext: &str,
    orientation: Option<i64>,
    dest: &Path,
    et: &ExifTool,
    tmp: &Path,
) -> Result<(u32, u32)> {
    let img = load_image(src, ext, orientation, THUMB_SIZE, et, tmp)?;
    let (w, h) = (img.width(), img.height());
    let t = img.thumbnail(THUMB_SIZE, THUMB_SIZE);
    let bytes = encode_jpeg(&t, 80)?;
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(dest, bytes)?;
    Ok((w, h))
}

pub fn make_preview(src: &Path, ext: &str, orientation: Option<i64>, et: &ExifTool, tmp: &Path) -> Result<Vec<u8>> {
    let img = load_image(src, ext, orientation, 1600, et, tmp)?;
    let img = if img.width() > PREVIEW_SIZE || img.height() > PREVIEW_SIZE {
        img.resize(PREVIEW_SIZE, PREVIEW_SIZE, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    encode_jpeg(&img, 88)
}
