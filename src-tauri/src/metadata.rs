//! Turning ExifTool output into catalogue fields, and catalogue edits into ExifTool arguments.

use serde::Deserialize;
use serde_json::Value;

pub const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif", "avif",
];
pub const RAW_EXTS: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef", "srw",
    "x3f", "3fr", "iiq", "erf", "mef", "mos", "kdc", "dcr", "rwl",
];

pub fn is_supported(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    IMAGE_EXTS.contains(&e.as_str()) || RAW_EXTS.contains(&e.as_str())
}

pub fn is_raw(ext: &str) -> bool {
    RAW_EXTS.contains(&ext.to_ascii_lowercase().as_str())
}

/// Formats the webview can display directly.
pub fn browser_native(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    let mut ok = matches!(e.as_str(), "jpg" | "jpeg" | "jpe" | "png" | "webp" | "gif" | "bmp" | "avif");
    if cfg!(target_os = "macos") {
        ok |= matches!(e.as_str(), "heic" | "heif");
    }
    ok
}

#[derive(Debug, Default, Clone)]
pub struct Meta {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub orientation: Option<i64>,
    pub taken_at: Option<String>,
    pub rating: i64,
    pub label: Option<String>,
    pub title: Option<String>,
    pub caption: Option<String>,
    pub keywords: Vec<String>,
    pub creator: Option<String>,
    pub copyright: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub alt: Option<f64>,
    pub place: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<i64>,
    pub fnumber: Option<f64>,
    pub exposure: Option<f64>,
    pub focal: Option<f64>,
}

fn s(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        match v.get(*k) {
            Some(Value::String(x)) if !x.trim().is_empty() => return Some(x.trim().to_string()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            Some(Value::Array(a)) if !a.is_empty() => {
                let parts: Vec<String> = a
                    .iter()
                    .filter_map(|x| match x {
                        Value::String(s) => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
                    .collect();
                if !parts.is_empty() {
                    return Some(parts.join(", "));
                }
            }
            _ => {}
        }
    }
    None
}

fn f(v: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        match v.get(*k) {
            Some(Value::Number(n)) => return n.as_f64(),
            Some(Value::String(x)) => {
                if let Ok(n) = x.trim().parse::<f64>() {
                    return Some(n);
                }
            }
            _ => {}
        }
    }
    None
}

fn list(v: &Value, keys: &[&str]) -> Vec<String> {
    for k in keys {
        match v.get(*k) {
            Some(Value::Array(a)) if !a.is_empty() => {
                return a
                    .iter()
                    .filter_map(|x| match x {
                        Value::String(s) => Some(s.trim().to_string()),
                        Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            Some(Value::String(x)) if !x.trim().is_empty() => {
                return x.split([',', ';']).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
            }
            Some(Value::Number(n)) => return vec![n.to_string()],
            _ => {}
        }
    }
    vec![]
}

/// ExifTool dates look like "2023:07:14 18:22:05+02:00"; store ISO-like "2023-07-14T18:22:05".
pub fn normalize_date(d: &str) -> Option<String> {
    let d = d.trim();
    if d.len() < 10 || d.starts_with("0000") {
        return None;
    }
    let date = d[..10].replace(':', "-");
    let time = if d.len() >= 19 { d[11..19].to_string() } else { "00:00:00".into() };
    if !date.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return None;
    }
    Some(format!("{date}T{time}"))
}

pub fn parse(v: &Value) -> Meta {
    let mut m = Meta::default();
    m.width = f(v, &["ExifImageWidth", "ImageWidth"]).map(|x| x as i64);
    m.height = f(v, &["ExifImageHeight", "ImageHeight"]).map(|x| x as i64);
    m.orientation = f(v, &["Orientation"]).map(|x| x as i64);
    if let Some(o) = m.orientation {
        if (5..=8).contains(&o) {
            std::mem::swap(&mut m.width, &mut m.height);
        }
    }
    m.taken_at = s(v, &["DateTimeOriginal", "CreateDate", "DateCreated"]).and_then(|d| normalize_date(&d));
    m.rating = f(v, &["Rating"]).map(|r| r.clamp(-1.0, 5.0) as i64).unwrap_or(0).max(0);
    m.label = s(v, &["Label"]);
    m.title = s(v, &["Title", "ObjectName"]);
    m.caption = s(v, &["Description", "Caption-Abstract", "ImageDescription"]);
    let mut kw = list(v, &["Subject"]);
    for k in list(v, &["Keywords"]) {
        if !kw.iter().any(|x| x.eq_ignore_ascii_case(&k)) {
            kw.push(k);
        }
    }
    m.keywords = kw;
    m.creator = s(v, &["Creator", "Artist", "By-line"]);
    m.copyright = s(v, &["Rights", "Copyright", "CopyrightNotice"]);
    m.lat = f(v, &["GPSLatitude"]).filter(|x| x.abs() <= 90.0);
    m.lon = f(v, &["GPSLongitude"]).filter(|x| x.abs() <= 180.0);
    if m.lat.is_none() || m.lon.is_none() || (m.lat == Some(0.0) && m.lon == Some(0.0)) {
        m.lat = None;
        m.lon = None;
    }
    m.alt = f(v, &["GPSAltitude"]);
    let place: Vec<String> = [
        s(v, &["Sub-location", "Location"]),
        s(v, &["City"]),
        s(v, &["State", "Province-State"]),
        s(v, &["Country", "Country-PrimaryLocationName"]),
    ]
    .into_iter()
    .flatten()
    .collect();
    m.place = if place.is_empty() { None } else { Some(place.join(", ")) };
    m.make = s(v, &["Make"]);
    m.model = s(v, &["Model"]);
    m.lens = s(v, &["LensModel", "Lens", "LensID"]);
    m.iso = f(v, &["ISO"]).map(|x| x as i64);
    m.fnumber = f(v, &["FNumber"]);
    m.exposure = f(v, &["ExposureTime"]);
    m.focal = f(v, &["FocalLength"]);
    m
}

/// Overlay XMP sidecar values (Lightroom / Bridge style) on top of the image's own.
pub fn merge_sidecar(base: &mut Meta, side: &Meta) {
    if side.rating > 0 {
        base.rating = side.rating;
    }
    macro_rules! take {
        ($($f:ident),*) => { $( if side.$f.is_some() { base.$f = side.$f.clone(); } )* };
    }
    take!(label, title, caption, creator, copyright, lat, lon, alt, place);
    // A sidecar orientation (e.g. a RAW rotated in Lumio or Lightroom) wins.
    if let Some(o) = side.orientation.filter(|o| (1..=8).contains(o)) {
        if is_quarter_turn(base.orientation) != is_quarter_turn(Some(o)) {
            std::mem::swap(&mut base.width, &mut base.height);
        }
        base.orientation = Some(o);
    }
    if !side.keywords.is_empty() {
        base.keywords = side.keywords.clone();
    }
}

/// A partial edit coming from the UI. Only fields that are `Some` are written.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Edit {
    pub rating: Option<i64>,
    pub label: Option<String>,
    pub title: Option<String>,
    pub caption: Option<String>,
    pub keywords: Option<Vec<String>>,
    /// Keywords to add / remove without replacing the list (multi-selection edits).
    pub add_keywords: Option<Vec<String>>,
    pub remove_keywords: Option<Vec<String>>,
    pub creator: Option<String>,
    pub copyright: Option<String>,
    /// `[lat, lon]` sets a location.
    pub gps: Option<[f64; 2]>,
    /// Remove the location.
    pub clear_gps: bool,
}

impl Edit {
    /// `Some(Some((lat, lon)))` sets a location, `Some(None)` removes it.
    pub fn gps_change(&self) -> Option<Option<(f64, f64)>> {
        if self.clear_gps {
            Some(None)
        } else {
            self.gps.map(|g| Some((g[0], g[1])))
        }
    }
}

/// Build ExifTool assignments for an edit. `xmp_only` is used for sidecar files.
pub fn write_args(e: &Edit, final_keywords: Option<&[String]>, xmp_only: bool) -> Vec<String> {
    let mut a = vec![];
    if let Some(r) = e.rating {
        a.push(format!("-XMP-xmp:Rating={}", r.clamp(0, 5)));
        if !xmp_only {
            // Windows Explorer reads this one.
            a.push(format!("-XMP-microsoft:RatingPercent={}", [0, 1, 25, 50, 75, 99][r.clamp(0, 5) as usize]));
        }
    }
    if let Some(l) = &e.label {
        a.push(format!("-XMP-xmp:Label={l}"));
    }
    if let Some(t) = &e.title {
        a.push(format!("-XMP-dc:Title={t}"));
        if !xmp_only {
            a.push(format!("-IPTC:ObjectName={}", truncate(t, 64)));
        }
    }
    if let Some(c) = &e.caption {
        a.push(format!("-XMP-dc:Description={c}"));
        if !xmp_only {
            a.push(format!("-IPTC:Caption-Abstract={}", truncate(c, 2000)));
            a.push(format!("-EXIF:ImageDescription={c}"));
        }
    }
    if let Some(k) = final_keywords {
        a.push("-XMP-dc:Subject=".into());
        if !xmp_only {
            a.push("-IPTC:Keywords=".into());
        }
        for kw in k {
            a.push(format!("-XMP-dc:Subject={kw}"));
            if !xmp_only {
                a.push(format!("-IPTC:Keywords={}", truncate(kw, 64)));
            }
        }
    }
    if let Some(c) = &e.creator {
        a.push(format!("-XMP-dc:Creator={c}"));
        if !xmp_only {
            a.push(format!("-EXIF:Artist={c}"));
            a.push(format!("-IPTC:By-line={}", truncate(c, 32)));
        }
    }
    if let Some(c) = &e.copyright {
        a.push(format!("-XMP-dc:Rights={c}"));
        if !xmp_only {
            a.push(format!("-EXIF:Copyright={c}"));
            a.push(format!("-IPTC:CopyrightNotice={}", truncate(c, 128)));
        }
    }
    if let Some(g) = &e.gps_change() {
        let prefix = if xmp_only { "-XMP:" } else { "-" };
        match g {
            Some((lat, lon)) => {
                let wild = if xmp_only { "" } else { "*" };
                a.push(format!("{prefix}GPSLatitude{wild}={lat:.7}"));
                a.push(format!("{prefix}GPSLongitude{wild}={lon:.7}"));
                if !xmp_only {
                    a.push(format!("-GPSLatitudeRef*={lat:.7}"));
                    a.push(format!("-GPSLongitudeRef*={lon:.7}"));
                }
            }
            None => {
                if xmp_only {
                    a.push("-XMP:GPSLatitude=".into());
                    a.push("-XMP:GPSLongitude=".into());
                } else {
                    a.push("-gps:all=".into());
                    a.push("-XMP:GPSLatitude=".into());
                    a.push("-XMP:GPSLongitude=".into());
                }
            }
        }
    }
    a
}

fn truncate(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Orientations 5–8 show the stored image turned by a quarter, so width and height swap.
pub fn is_quarter_turn(o: Option<i64>) -> bool {
    matches!(o, Some(5..=8))
}

/// EXIF orientation as "mirror horizontally first (f), then turn clockwise r quarters".
fn orientation_parts(o: i64) -> (u8, u8) {
    match o {
        2 => (1, 0),
        3 => (0, 2),
        4 => (1, 2),
        5 => (1, 3),
        6 => (0, 1),
        7 => (1, 1),
        8 => (0, 3),
        _ => (0, 0),
    }
}

fn orientation_from_parts(f: u8, r: u8) -> i64 {
    match (f % 2, r % 4) {
        (0, 0) => 1,
        (1, 0) => 2,
        (0, 2) => 3,
        (1, 2) => 4,
        (1, 3) => 5,
        (0, 1) => 6,
        (1, 1) => 7,
        (0, 3) => 8,
        _ => 1,
    }
}

/// Apply a turn or flip, as seen on screen, to an EXIF orientation.
/// `op` is "right" (90° clockwise), "left", "flip_h" (mirror left–right) or "flip_v".
pub fn transform_orientation(current: Option<i64>, op: &str) -> Option<i64> {
    let (f, r) = orientation_parts(current.unwrap_or(1));
    let (f, r) = match op {
        "right" => (f, (r + 1) % 4),
        "left" => (f, (r + 3) % 4),
        "flip_h" => (1 - f, (4 - r) % 4),
        "flip_v" => (1 - f, (6 - r) % 4),
        _ => return None,
    };
    Some(orientation_from_parts(f, r))
}

#[cfg(test)]
mod orientation_tests {
    use super::*;

    #[test]
    fn four_turns_come_back() {
        for o in 1..=8 {
            let mut x = Some(o);
            for _ in 0..4 {
                x = transform_orientation(x, "right");
            }
            assert_eq!(x, Some(o));
            assert_eq!(transform_orientation(transform_orientation(Some(o), "left"), "right"), Some(o));
            assert_eq!(transform_orientation(transform_orientation(Some(o), "flip_h"), "flip_h"), Some(o));
            assert_eq!(transform_orientation(transform_orientation(Some(o), "flip_v"), "flip_v"), Some(o));
        }
    }

    #[test]
    fn known_values() {
        assert_eq!(transform_orientation(Some(1), "right"), Some(6));
        assert_eq!(transform_orientation(Some(1), "left"), Some(8));
        assert_eq!(transform_orientation(Some(6), "right"), Some(3));
        assert_eq!(transform_orientation(Some(1), "flip_h"), Some(2));
        assert_eq!(transform_orientation(Some(1), "flip_v"), Some(4));
        assert_eq!(transform_orientation(Some(6), "flip_h"), Some(5));
        assert_eq!(transform_orientation(Some(6), "flip_v"), Some(7));
    }
}
