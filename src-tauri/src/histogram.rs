//! Colour histogram for the info panel.
//!
//! Counted from the picture Lumio already keeps for each photo (the sharper offline
//! preview when there is one, otherwise the thumbnail), so it is quick, works with the
//! drive unplugged and never decodes a large RAW file. A few hundred thousand pixels is
//! plenty for the shape of a histogram.

use serde::Serialize;
use std::path::Path;

/// Pixel counts for each of the 256 levels, per channel, plus luminance.
#[derive(Debug, Serialize, PartialEq)]
pub struct Histogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    pub l: Vec<u32>,
    /// Share of pixels clipped to pure black / pure white in any channel (0..1).
    pub shadows_clipped: f32,
    pub highlights_clipped: f32,
}

/// Counts levels in an RGB8 buffer (3 bytes per pixel).
pub fn from_rgb(pixels: &[u8]) -> Histogram {
    let (mut r, mut g, mut b, mut l) = (
        vec![0u32; 256],
        vec![0u32; 256],
        vec![0u32; 256],
        vec![0u32; 256],
    );
    let (mut lo, mut hi, mut n) = (0u32, 0u32, 0u32);
    for px in pixels.chunks_exact(3) {
        let (pr, pg, pb) = (px[0], px[1], px[2]);
        r[pr as usize] += 1;
        g[pg as usize] += 1;
        b[pb as usize] += 1;
        // Rec. 709 luma weights, in integers.
        let y = (2126 * pr as u32 + 7152 * pg as u32 + 722 * pb as u32 + 5000) / 10000;
        l[y.min(255) as usize] += 1;
        if pr == 0 && pg == 0 && pb == 0 {
            lo += 1;
        }
        if pr == 255 || pg == 255 || pb == 255 {
            hi += 1;
        }
        n += 1;
    }
    let share = |x: u32| if n == 0 { 0.0 } else { x as f32 / n as f32 };
    Histogram {
        r,
        g,
        b,
        l,
        shadows_clipped: share(lo),
        highlights_clipped: share(hi),
    }
}

/// Reads an image file (JPEG from Lumio's own folders) and counts its levels.
pub fn from_file(path: &Path) -> anyhow::Result<Histogram> {
    let img = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?;
    Ok(from_rgb(img.to_rgb8().as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_each_channel_and_luminance() {
        // black, white, pure red, mid grey
        let px = [0, 0, 0, 255, 255, 255, 255, 0, 0, 128, 128, 128];
        let h = from_rgb(&px);
        assert_eq!(h.r[0], 1);
        assert_eq!(h.r[255], 2);
        assert_eq!(h.r[128], 1);
        assert_eq!(h.g[0], 2);
        assert_eq!(h.b[255], 1);
        assert_eq!(h.l[0], 1);
        assert_eq!(h.l[255], 1);
        assert_eq!(h.l[128], 1);
        assert_eq!(h.l[54], 1); // red: 0.2126 * 255
        assert_eq!(h.l.iter().sum::<u32>(), 4);
        assert!((h.shadows_clipped - 0.25).abs() < 1e-6);
        assert!((h.highlights_clipped - 0.5).abs() < 1e-6);
    }

    #[test]
    fn empty_image_has_no_clipping() {
        let h = from_rgb(&[]);
        assert_eq!(h.shadows_clipped, 0.0);
        assert_eq!(h.l.iter().sum::<u32>(), 0);
    }

    #[test]
    fn reads_a_jpeg() {
        let dir = std::env::temp_dir().join(format!("lumio-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.jpg");
        image::RgbImage::from_pixel(8, 8, image::Rgb([255, 255, 255]))
            .save(&p)
            .unwrap();
        let h = from_file(&p).unwrap();
        assert_eq!(h.l.iter().sum::<u32>(), 64);
        assert!(h.l[250..].iter().sum::<u32>() == 64);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
