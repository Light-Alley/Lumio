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

/// The sharper preview Lumio keeps for offline viewing (same layout as thumbnails,
/// in its own folder). See `offline.rs` for which photos get one.
pub fn offline_preview_path(dir: &Path, id: i64) -> PathBuf {
    thumb_path(dir, id)
}

fn decode_with_orientation(reader: ImageReader<impl std::io::BufRead + std::io::Seek>) -> Result<DynamicImage> {
    let mut decoder = reader.with_guessed_format()?.into_decoder()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// Decode an embedded preview. A preview that carries its own orientation tag describes
/// itself; one without (the usual case) is stored the way the sensor saw it, so the
/// photo's orientation applies. Never both, which would turn it twice.
fn decode_preview(bytes: Vec<u8>, photo_orientation: Option<i64>) -> Result<DynamicImage> {
    let mut decoder = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?.into_decoder()?;
    let own = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(if own == Orientation::NoTransforms { orientation_from_exif(photo_orientation) } else { own });
    Ok(img)
}

/// Decode a file the way it should be shown (its EXIF orientation applied).
pub fn open_upright(path: &Path) -> Result<DynamicImage> {
    decode_with_orientation(ImageReader::open(path)?)
}

/// Writes `bytes` to `dest` through a temporary file in the same folder, so nobody ever
/// reads a half-written file (two writers of the same file each use their own name).
pub fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    let part = dest.with_extension(format!("{}.part", crate::secure::random_hex(4)?));
    if let Err(e) = std::fs::write(&part, bytes).and_then(|_| std::fs::rename(&part, dest)) {
        let _ = std::fs::remove_file(&part);
        return Err(e.into());
    }
    Ok(())
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
        Ok(Some(decode_preview(bytes, orientation)?))
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
    use std::time::{Duration, Instant};
    let out = tmp.join(format!("sips-{}.jpg", crate::secure::random_hex(8)?));
    let mut child = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg", "-Z", &PREVIEW_SIZE.to_string()])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    // A file on a network drive that stopped answering must not hold up indexing.
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&out);
            return Err(anyhow!("sips timed out"));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !status.success() {
        let _ = std::fs::remove_file(&out);
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

/// What `make_thumb` made.
pub struct Made {
    /// Size of the source image.
    pub width: u32,
    pub height: u32,
    /// Whether the offline preview was saved, when one was asked for.
    pub preview: Option<bool>,
}

/// Create a thumbnail file, and optionally an offline preview (`dest`, long side) from
/// the same decoded image, so the file is only read once.
pub fn make_thumb(
    src: &Path,
    ext: &str,
    orientation: Option<i64>,
    dest: &Path,
    preview: Option<(&Path, u32)>,
    et: &ExifTool,
    tmp: &Path,
) -> Result<Made> {
    if metadata::is_video(ext) {
        let (width, height) = write_thumb(&video_frame(src, et, tmp)?, dest)?;
        return Ok(Made { width, height, preview: None });
    }
    let want = preview.map(|(_, s)| s).unwrap_or(THUMB_SIZE);
    let img = load_image(src, ext, orientation, want, et, tmp)?;
    let (width, height) = write_thumb(&img, dest)?;
    let preview = preview.map(|(p, size)| match write_preview(&img, p, size) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("offline preview failed for {}: {e:#}", src.display());
            false
        }
    });
    Ok(Made { width, height, preview })
}

/// Make just the offline preview (for photos indexed before previews were kept).
pub fn make_offline_preview(src: &Path, ext: &str, orientation: Option<i64>, dest: &Path, size: u32, et: &ExifTool, tmp: &Path) -> Result<()> {
    let img = load_image(src, ext, orientation, size, et, tmp)?;
    write_preview(&img, dest, size)
}

/// Save a downscaled JPEG. Written to a temporary name first, so a preview is never
/// left half-written if Lumio quits or the disk fills up.
fn write_preview(img: &DynamicImage, dest: &Path, size: u32) -> Result<()> {
    let img = if img.width() > size || img.height() > size {
        img.resize(size, size, image::imageops::FilterType::CatmullRom)
    } else {
        img.clone()
    };
    // High quality previews (for full screen on sharp displays) also compress less.
    write_atomic(dest, &encode_jpeg(&img, if size >= 2400 { 90 } else { 82 })?)
}

fn write_thumb(img: &DynamicImage, dest: &Path) -> Result<(u32, u32)> {
    let (w, h) = (img.width(), img.height());
    let t = img.thumbnail(THUMB_SIZE, THUMB_SIZE);
    write_atomic(dest, &encode_jpeg(&t, 80)?)?;
    Ok((w, h))
}

/// Save a video frame the app's own player captured (used where the system has no
/// video thumbnailer Lumio can call: Windows, Linux, and files macOS can't preview).
pub fn save_frame(bytes: &[u8], dest: &Path) -> Result<(u32, u32)> {
    let img = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?.decode()?;
    write_thumb(&img, dest)
}

/// A still for a video thumbnail without bundling a video decoder: a preview the camera
/// embedded in the file, or on macOS the system's Quick Look thumbnail. Elsewhere the
/// app captures a frame with the webview's player instead (see `save_frame`).
fn video_frame(src: &Path, et: &ExifTool, tmp: &Path) -> Result<DynamicImage> {
    if et.available() {
        if let Ok(Some(bytes)) = et.extract_preview(src, tmp) {
            if let Ok(img) = decode_preview(bytes, None) {
                return Ok(img);
            }
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(img) = quicklook_thumb(src, tmp) {
        return Ok(img);
    }
    Err(anyhow!("no video thumbnail available for {}", src.display()))
}

/// Ask Quick Look for a thumbnail (macOS decodes H.264, HEVC, ProRes and so on).
#[cfg(target_os = "macos")]
fn quicklook_thumb(src: &Path, tmp: &Path) -> Result<DynamicImage> {
    use std::time::{Duration, Instant};
    let dir = tmp.join(format!("ql-{}", crate::secure::random_hex(8)?));
    std::fs::create_dir_all(&dir)?;
    let result = (|| -> Result<DynamicImage> {
        let mut child = std::process::Command::new("/usr/bin/qlmanage")
            .args(["-t", "-s", &THUMB_SIZE.to_string(), "-o"])
            .arg(&dir)
            .arg(src)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        // A file on a network drive that stopped answering must not hold up indexing.
        let start = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if start.elapsed() > Duration::from_secs(30) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!("Quick Look timed out"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let out = std::fs::read_dir(&dir)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
            .ok_or_else(|| anyhow!("Quick Look made no thumbnail"))?;
        Ok(ImageReader::open(out)?.with_guessed_format()?.decode()?)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    /// A 20×10 JPEG, optionally tagged with an EXIF orientation.
    fn jpeg(orientation: Option<u8>) -> Vec<u8> {
        let img = image::RgbImage::new(20, 10);
        let mut buf = Vec::new();
        let mut enc = JpegEncoder::new_with_quality(&mut buf, 80);
        if let Some(o) = orientation {
            // Big-endian TIFF header, one IFD entry: Orientation (0x0112), SHORT, 1, o.
            let exif = vec![
                0x4D, 0x4D, 0x00, 0x2A, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, o, 0, 0, 0, 0, 0, 0,
            ];
            enc.set_exif_metadata(exif).unwrap();
        }
        enc.write_image(img.as_raw(), 20, 10, image::ExtendedColorType::Rgb8).unwrap();
        buf
    }

    #[test]
    fn untagged_preview_takes_the_photos_orientation() {
        let img = decode_preview(jpeg(None), Some(6)).unwrap();
        assert_eq!((img.width(), img.height()), (10, 20));
    }

    #[test]
    fn tagged_preview_is_not_turned_twice() {
        // The preview says "turn me" itself; the photo says the same. Turn once.
        let img = decode_preview(jpeg(Some(6)), Some(6)).unwrap();
        assert_eq!((img.width(), img.height()), (10, 20));
        // A preview with its own (different) turn follows its tag, not the photo's.
        let img = decode_preview(jpeg(Some(3)), Some(6)).unwrap();
        assert_eq!((img.width(), img.height()), (20, 10));
    }

    #[test]
    fn portrait_jpegs_open_upright() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("p.jpg");
        std::fs::write(&f, jpeg(Some(6))).unwrap();
        let img = open_upright(&f).unwrap();
        assert_eq!((img.width(), img.height()), (10, 20));
    }

    #[test]
    fn atomic_writes_leave_no_parts() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("ab").join("1.jpg");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(f.parent().unwrap()).unwrap().count(), 1);
    }
}
