//! Turning ExifTool output into catalogue fields, and catalogue edits into ExifTool arguments.

use serde::Deserialize;
use serde_json::Value;

pub const IMAGE_EXTS: &[&str] =
    &["jpg", "jpeg", "jpe", "png", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif", "avif"];
pub const RAW_EXTS: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef", "srw", "x3f", "3fr",
    "iiq", "erf", "mef", "mos", "kdc", "dcr", "rwl",
];

/// Video files. Frameloft catalogues them like photos and plays them in the webview's own
/// player, so which ones actually play depends on the system (see docs/video.md).
pub const VIDEO_EXTS: &[&str] =
    &["mp4", "m4v", "mov", "webm", "mkv", "avi", "3gp", "mts", "m2ts", "mpg", "mpeg", "wmv", "ogv"];

pub fn is_supported(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    IMAGE_EXTS.contains(&e.as_str()) || RAW_EXTS.contains(&e.as_str()) || VIDEO_EXTS.contains(&e.as_str())
}

pub fn is_video(ext: &str) -> bool {
    VIDEO_EXTS.contains(&ext.to_ascii_lowercase().as_str())
}

/// Videos ExifTool can't write to directly; their metadata goes into an XMP sidecar.
pub fn video_needs_sidecar(ext: &str) -> bool {
    is_video(ext) && !matches!(ext.to_ascii_lowercase().as_str(), "mp4" | "m4v" | "mov" | "3gp")
}

/// The MIME type a video is served with. Chromium-based webviews (Windows) don't claim
/// QuickTime, but play most .mov files when told they are MP4.
pub fn video_mime(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" if cfg!(target_os = "macos") => "video/quicktime",
        "mov" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "3gp" => "video/3gpp",
        "mts" | "m2ts" => "video/mp2t",
        "mpg" | "mpeg" => "video/mpeg",
        "wmv" => "video/x-ms-wmv",
        "ogv" => "video/ogg",
        _ => "application/octet-stream",
    }
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
    /// Lightroom-style keyword paths ("Places|France|Paris"), from lr:hierarchicalSubject.
    pub hier_keywords: Vec<String>,
    /// Whether the file actually holds a rating, label or keyword tag. A sidecar that says
    /// "no rating" (because it was cleared in Lightroom) must win over the RAW's own value.
    pub has_rating: bool,
    pub has_label: bool,
    pub has_keywords: bool,
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
    /// Length of a video in seconds.
    pub duration: Option<f64>,
    /// Rotation a video is played with (QuickTime "Rotation": 0, 90, 180 or 270).
    pub video_rotation: Option<i64>,
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

/// A list tag whose items may themselves contain commas (keyword paths): a single string
/// is one item, not a comma-separated list.
fn list_exact(v: &Value, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(x)) if !x.trim().is_empty() => vec![x.trim().to_string()],
        _ => vec![],
    }
}

/// ExifTool dates look like "2023:07:14 18:22:05+02:00"; store ISO-like "2023-07-14T18:22:05".
/// Anything else (text in other scripts, half-dates) gives `None`, never a panic.
pub fn normalize_date(d: &str) -> Option<String> {
    let d = d.trim();
    if d.starts_with("0000") {
        return None;
    }
    let date = d.get(..10)?.replace(':', "-");
    let b = date.as_bytes();
    let shape = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if !shape {
        return None;
    }
    let time = match d.get(11..19) {
        Some(t)
            if t.len() == 8
                && t.bytes()
                    .enumerate()
                    .all(|(i, c)| if i == 2 || i == 5 { c == b':' } else { c.is_ascii_digit() }) =>
        {
            t.to_string()
        }
        _ => "00:00:00".into(),
    };
    Some(format!("{date}T{time}"))
}

pub fn parse(v: &Value) -> Meta {
    let mut m = Meta {
        width: f(v, &["ExifImageWidth", "ImageWidth"]).map(|x| x as i64),
        height: f(v, &["ExifImageHeight", "ImageHeight"]).map(|x| x as i64),
        orientation: f(v, &["Orientation"]).map(|x| x as i64),
        ..Default::default()
    };
    if let Some(o) = m.orientation {
        if (5..=8).contains(&o) {
            std::mem::swap(&mut m.width, &mut m.height);
        }
    }
    m.taken_at = s(v, &["DateTimeOriginal", "CreateDate", "DateCreated"]).and_then(|d| normalize_date(&d));
    m.rating = f(v, &["Rating"]).map(|r| r.clamp(-1.0, 5.0) as i64).unwrap_or(0).max(0);
    m.has_rating = v.get("Rating").is_some();
    m.label = s(v, &["Label"]);
    m.has_label = v.get("Label").is_some();
    m.title = s(v, &["Title", "ObjectName"]);
    m.caption = s(v, &["Description", "Caption-Abstract", "ImageDescription"]);
    let mut kw = list(v, &["Subject"]);
    for k in list(v, &["Keywords"]) {
        if !kw.iter().any(|x| x.eq_ignore_ascii_case(&k)) {
            kw.push(k);
        }
    }
    m.hier_keywords = list_exact(v, "HierarchicalSubject");
    // Files tagged only with keyword paths still show their keywords.
    for h in &m.hier_keywords {
        if let Some(leaf) = split_path(h).last() {
            if !kw.iter().any(|x| x.eq_ignore_ascii_case(leaf)) {
                kw.push(leaf.to_string());
            }
        }
    }
    m.has_keywords = ["Subject", "Keywords", "HierarchicalSubject"].iter().any(|k| v.get(*k).is_some());
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
    m.duration = f(v, &["Duration"])
        .or_else(|| s(v, &["Duration"]).and_then(|d| parse_clock(&d)))
        .filter(|d| d.is_finite() && *d > 0.0);
    m.video_rotation = f(v, &["Rotation"]).map(|x| x as i64);
    m
}

/// "00:01:30.5" or "1:30" -> seconds (Matroska/WebM durations come as text).
fn parse_clock(t: &str) -> Option<f64> {
    t.trim().split(':').try_fold(0.0, |acc, part| part.trim().parse::<f64>().ok().map(|x| acc * 60.0 + x))
}

/// Videos record their size unrotated plus a rotation to play them with (phones filming
/// upright store 90). Swap width and height so the catalogue holds the size as seen.
pub fn fix_video_size(m: &mut Meta) {
    if matches!(m.video_rotation, Some(90) | Some(270)) {
        std::mem::swap(&mut m.width, &mut m.height);
    }
}

/// Overlay XMP sidecar values (Lightroom / Bridge style) on top of the image's own.
///
/// Rating, label and keywords follow the sidecar whenever it has the tag at all, so that
/// clearing them in Lightroom or Capture One (rating 0, no label) is not undone by the
/// value the camera wrote into the RAW file.
pub fn merge_sidecar(base: &mut Meta, side: &Meta) {
    if side.has_rating || side.rating > 0 {
        base.rating = side.rating;
    }
    if side.has_label {
        base.label = side.label.clone();
    }
    macro_rules! take {
        ($($f:ident),*) => { $( if side.$f.is_some() { base.$f = side.$f.clone(); } )* };
    }
    take!(label, title, caption, creator, copyright, lat, lon, alt, place);
    // A sidecar orientation (e.g. a RAW rotated in Frameloft or Lightroom) wins.
    if let Some(o) = side.orientation.filter(|o| (1..=8).contains(o)) {
        if is_quarter_turn(base.orientation) != is_quarter_turn(Some(o)) {
            std::mem::swap(&mut base.width, &mut base.height);
        }
        base.orientation = Some(o);
    }
    if side.has_keywords || !side.keywords.is_empty() {
        base.keywords = side.keywords.clone();
        base.hier_keywords = side.hier_keywords.clone();
    }
}

/// Split a keyword path. Lightroom and Capture One store "Places|France|Paris"; people
/// often type "Places > France > Paris", which Frameloft accepts too.
pub fn split_path(k: &str) -> Vec<&str> {
    let parts: Vec<&str> = if k.contains('|') { k.split('|').collect() } else { k.split(" > ").collect() };
    parts.into_iter().map(str::trim).filter(|p| !p.is_empty()).collect()
}

fn has_ci(list: &[String], k: &str) -> bool {
    list.iter().any(|x| x.eq_ignore_ascii_case(k))
}

fn push_ci(list: &mut Vec<String>, k: &str) {
    if !k.is_empty() && !has_ci(list, k) {
        list.push(k.to_string());
    }
}

/// The keywords to write, as Lightroom and Capture One expect them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeywordPlan {
    /// dc:subject (and IPTC Keywords): every keyword, including the parents of a path.
    pub subject: Vec<String>,
    /// lr:hierarchicalSubject: the keyword paths.
    pub hierarchy: Vec<String>,
}

/// Work out dc:subject and lr:hierarchicalSubject for a new keyword list.
///
/// `wanted` is the list the person ended up with; entries may be paths ("Places|France|Paris").
/// `removed` are keywords they took off. `old_hier` is the file's current
/// lr:hierarchicalSubject. A path is kept while its last keyword is still wanted and none
/// of its keywords was removed; otherwise Lightroom would bring the keyword back the next
/// time it reads the file. Parents that only came from a dropped path go with it, as they
/// do in Lightroom.
pub fn plan_keywords(wanted: &[String], removed: &[String], old_hier: &[String]) -> KeywordPlan {
    let mut flat: Vec<String> = vec![];
    let mut new_paths: Vec<Vec<String>> = vec![];
    for w in wanted {
        let parts = split_path(w);
        match parts.len() {
            0 => {}
            1 => push_ci(&mut flat, parts[0]),
            _ => new_paths.push(parts.iter().map(|s| s.to_string()).collect()),
        }
    }
    let mut hierarchy: Vec<String> = vec![];
    let mut kept: Vec<Vec<String>> = vec![];
    let mut dropped: Vec<Vec<String>> = vec![];
    for h in old_hier {
        let parts: Vec<String> = split_path(h).iter().map(|s| s.to_string()).collect();
        let Some(leaf) = parts.last() else { continue };
        let leaf_wanted =
            has_ci(&flat, leaf) || new_paths.iter().any(|p| p.last().is_some_and(|l| l.eq_ignore_ascii_case(leaf)));
        if leaf_wanted && !parts.iter().any(|p| has_ci(removed, p)) {
            push_ci(&mut hierarchy, &parts.join("|"));
            kept.push(parts);
        } else {
            dropped.push(parts);
        }
    }
    for p in &new_paths {
        push_ci(&mut hierarchy, &p.join("|"));
    }
    // Parents of dropped paths that nothing else still needs.
    let needed = |k: &str| kept.iter().chain(new_paths.iter()).any(|p| has_ci(p, k));
    let orphan =
        |k: &str| !needed(k) && dropped.iter().any(|p| p[..p.len() - 1].iter().any(|x| x.eq_ignore_ascii_case(k)));
    let mut subject: Vec<String> = vec![];
    for k in &flat {
        if !orphan(k) {
            push_ci(&mut subject, k);
        }
    }
    for p in kept.iter().chain(new_paths.iter()) {
        for k in p {
            push_ci(&mut subject, k);
        }
    }
    KeywordPlan { subject, hierarchy }
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
/// `keywords` is the result of [`plan_keywords`] when the keywords change; the
/// hierarchy is written only when there is one (or was one to clear).
pub fn write_args(e: &Edit, keywords: Option<(&KeywordPlan, bool)>, xmp_only: bool) -> Vec<String> {
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
    if let Some((plan, had_hier)) = keywords {
        let k = &plan.subject;
        a.push("-XMP-dc:Subject=".into());
        if !xmp_only {
            a.push("-IPTC:Keywords=".into());
        }
        if had_hier || !plan.hierarchy.is_empty() {
            a.push("-XMP-lr:HierarchicalSubject=".into());
            for h in &plan.hierarchy {
                a.push(format!("-XMP-lr:HierarchicalSubject={h}"));
            }
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

/// Every tag that says where a photo was taken: GPS coordinates and the IPTC/XMP place
/// names. Used when exporting or sharing "without location".
pub const LOCATION_TAGS: &[&str] = &[
    "GPS:all",
    "XMP:GPS*",
    "XMP-exif:GPS*",
    "IPTC:City",
    "IPTC:Sub-location",
    "IPTC:Province-State",
    "IPTC:Country-PrimaryLocationName",
    "IPTC:Country-PrimaryLocationCode",
    "XMP-photoshop:City",
    "XMP-photoshop:State",
    "XMP-photoshop:Country",
    "XMP-iptcCore:Location",
    "XMP-iptcCore:CountryCode",
    "XMP-iptcExt:LocationCreated",
    "XMP-iptcExt:LocationShown",
    "XMP-iptcExt:City",
    "XMP-iptcExt:CountryName",
    "XMP-iptcExt:CountryCode",
    "XMP-iptcExt:ProvinceState",
    "XMP-iptcExt:Sublocation",
    "Keys:GPSCoordinates",
    "UserData:GPSCoordinates",
    "ItemList:GPSCoordinates",
    "Keys:LocationName",
    "QuickTime:LocationInformation",
];

/// ExifTool arguments removing every tag in [`LOCATION_TAGS`].
pub fn strip_location_args() -> Vec<String> {
    LOCATION_TAGS.iter().map(|t| format!("-{t}=")).collect()
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

#[cfg(test)]
mod video_tests {
    use super::*;

    #[test]
    fn videos_are_recognised() {
        assert!(is_supported("MP4") && is_video("mov") && !is_video("jpg"));
        assert!(video_needs_sidecar("mkv") && !video_needs_sidecar("mp4"));
    }

    #[test]
    fn upright_phone_video_swaps_size() {
        let v = serde_json::json!({"ImageWidth": 1920, "ImageHeight": 1080, "Rotation": 90, "Duration": 12.5});
        let mut m = parse(&v);
        fix_video_size(&mut m);
        assert_eq!((m.width, m.height, m.duration), (Some(1080), Some(1920), Some(12.5)));
    }

    #[test]
    fn webm_duration_as_text() {
        let m = parse(&serde_json::json!({"Duration": "00:01:30.008000000"}));
        assert!((m.duration.unwrap() - 90.008).abs() < 1e-6);
    }
}

#[cfg(test)]
mod keyword_tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn typed_paths_become_lightroom_paths() {
        let p = plan_keywords(&v(&["Holiday", "Places > France > Paris"]), &[], &[]);
        assert_eq!(p.subject, v(&["Holiday", "Places", "France", "Paris"]));
        assert_eq!(p.hierarchy, v(&["Places|France|Paris"]));
    }

    #[test]
    fn untouched_paths_are_kept() {
        let old = v(&["Places|France|Paris"]);
        let p = plan_keywords(&v(&["Places", "France", "Paris", "Holiday"]), &[], &old);
        assert_eq!(p.hierarchy, old);
        assert_eq!(p.subject, v(&["Places", "France", "Paris", "Holiday"]));
    }

    #[test]
    fn removing_a_keyword_drops_its_path_and_lonely_parents() {
        // Otherwise Lightroom would bring "Paris" back from lr:hierarchicalSubject.
        let old = v(&["Places|France|Paris", "Places|France|Lyon"]);
        let p = plan_keywords(&v(&["Places", "France", "Lyon", "Holiday"]), &v(&["Paris"]), &old);
        assert_eq!(p.hierarchy, v(&["Places|France|Lyon"]));
        assert_eq!(p.subject, v(&["Places", "France", "Lyon", "Holiday"]));

        let p = plan_keywords(&v(&["Places", "France", "Holiday"]), &v(&["Paris"]), &v(&["Places|France|Paris"]));
        assert!(p.hierarchy.is_empty());
        assert_eq!(p.subject, v(&["Holiday"]));
    }

    #[test]
    fn removing_a_parent_drops_the_path_but_keeps_the_keyword() {
        let p = plan_keywords(&v(&["Places", "Paris"]), &v(&["France"]), &v(&["Places|France|Paris"]));
        assert!(p.hierarchy.is_empty());
        assert_eq!(p.subject, v(&["Paris"]));
    }

    #[test]
    fn hierarchy_written_only_when_needed() {
        let e = Edit::default();
        let flat = plan_keywords(&v(&["A"]), &[], &[]);
        let args = write_args(&e, Some((&flat, false)), true);
        assert!(!args.iter().any(|a| a.contains("HierarchicalSubject")));
        let args = write_args(&e, Some((&flat, true)), true);
        assert!(args.contains(&"-XMP-lr:HierarchicalSubject=".to_string()));
    }

    #[test]
    fn sidecar_clears_win_over_the_raw_file() {
        let mut raw = parse(&serde_json::json!({"Rating": 3, "Label": "Red", "Subject": ["a"]}));
        let side = parse(&serde_json::json!({"Rating": 0, "Label": "", "Subject": []}));
        merge_sidecar(&mut raw, &side);
        assert_eq!((raw.rating, raw.label.clone(), raw.keywords.len()), (0, None, 0));
        // A sidecar without those tags leaves the file's values alone.
        let mut raw = parse(&serde_json::json!({"Rating": 3, "Label": "Red"}));
        merge_sidecar(&mut raw, &parse(&serde_json::json!({"Title": "x"})));
        assert_eq!((raw.rating, raw.label.as_deref()), (3, Some("Red")));
    }

    #[test]
    fn odd_dates_never_panic() {
        assert_eq!(normalize_date("2023:07:14 18:22:05+02:00").as_deref(), Some("2023-07-14T18:22:05"));
        assert_eq!(normalize_date("2023:07:14").as_deref(), Some("2023-07-14T00:00:00"));
        // Multi-byte text where byte 10 or 19 falls inside a character used to panic.
        assert_eq!(normalize_date("2023年07月14日 12:00:00"), None);
        assert_eq!(normalize_date("2023:07:14 1é:00:00").as_deref(), Some("2023-07-14T00:00:00"));
        assert_eq!(normalize_date("0000:00:00 00:00:00"), None);
        assert_eq!(normalize_date("abc"), None);
        assert_eq!(normalize_date("2023-7-14 xx"), None);
        assert_eq!(normalize_date(""), None);
    }

    #[test]
    fn keywords_from_paths_only() {
        let m = parse(&serde_json::json!({"HierarchicalSubject": "Places|France, south|Nice"}));
        assert_eq!(m.hier_keywords, v(&["Places|France, south|Nice"]));
        assert_eq!(m.keywords, v(&["Nice"]));
    }
}

/// Round trips through a real ExifTool (skipped when none is installed).
#[cfg(test)]
mod exiftool_tests {
    use super::*;
    use crate::exiftool::ExifTool;

    const LR_SIDECAR: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   xmp:Rating="4"
   xmp:Label="Red"
   crs:Exposure2012="+0.35">
   <dc:subject><rdf:Bag><rdf:li>Paris</rdf:li><rdf:li>France</rdf:li><rdf:li>Places</rdf:li><rdf:li>Holiday</rdf:li></rdf:Bag></dc:subject>
   <lr:hierarchicalSubject><rdf:Bag><rdf:li>Places|France|Paris</rdf:li></rdf:Bag></lr:hierarchicalSubject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
"#;

    #[test]
    fn stripping_location_removes_coordinates_and_place_names() {
        let Some(et) = crate::exiftool::test_tool() else { return };
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("place.jpg");
        image::RgbImage::new(8, 8).save(&f).unwrap();
        let set: Vec<String> = [
            "-GPSLatitude=64.1",
            "-GPSLatitudeRef=N",
            "-GPSLongitude=21.9",
            "-GPSLongitudeRef=W",
            "-IPTC:City=Reykjavik",
            "-IPTC:Province-State=Capital",
            "-IPTC:Country-PrimaryLocationName=Iceland",
            "-IPTC:Sub-location=Harbour",
            "-XMP-photoshop:City=Reykjavik",
            "-XMP-photoshop:State=Capital",
            "-XMP-photoshop:Country=Iceland",
            "-XMP-iptcCore:Location=Harbour",
            "-XMP-iptcCore:CountryCode=IS",
            "-XMP-dc:Title=Keep me",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        et.write(&f, &set).unwrap();
        let before = et.read_json(std::slice::from_ref(&f)).unwrap();
        assert_eq!(before[0]["City"].as_str(), Some("Reykjavik"));
        et.write(&f, &strip_location_args()).unwrap();
        let after = et.read_json(std::slice::from_ref(&f)).unwrap();
        let o = after[0].as_object().unwrap();
        for k in [
            "GPSLatitude",
            "GPSLongitude",
            "City",
            "State",
            "Country",
            "Location",
            "Sub-location",
            "Province-State",
            "Country-PrimaryLocationName",
        ] {
            assert!(!o.contains_key(k), "{k} survived: {:?}", o.get(k));
        }
        assert_eq!(o["Title"].as_str(), Some("Keep me"));
    }

    #[test]
    fn edits_a_lightroom_sidecar_without_losing_develop_settings() {
        let Some(et) = crate::exiftool::test_tool() else { return };
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let side = dir.join("IMG_1.xmp");
        std::fs::write(&side, LR_SIDECAR).unwrap();
        let read = |et: &ExifTool| parse(&et.read_json(std::slice::from_ref(&side)).unwrap()[0]);

        let m = read(&et);
        assert_eq!((m.rating, m.label.as_deref()), (4, Some("Red")));
        assert_eq!(m.hier_keywords, vec!["Places|France|Paris".to_string()]);

        // Take "Paris" off, add a path, change rating and clear the label.
        let wanted: Vec<String> =
            ["Places", "France", "Holiday", "People|Family"].iter().map(|s| s.to_string()).collect();
        let plan = plan_keywords(&wanted, &["Paris".to_string()], &m.hier_keywords);
        let edit = Edit { rating: Some(2), label: Some(String::new()), ..Default::default() };
        et.write(&side, &write_args(&edit, Some((&plan, true)), true)).unwrap();

        let m = read(&et);
        assert_eq!(m.rating, 2);
        assert!(m.has_rating);
        assert_eq!(m.label, None);
        assert_eq!(m.hier_keywords, vec!["People|Family".to_string()]);
        assert_eq!(m.keywords, vec!["Holiday".to_string(), "People".to_string(), "Family".to_string()]);
        let text = std::fs::read_to_string(&side).unwrap();
        assert!(text.contains("Exposure2012"), "Lightroom develop settings must survive:\n{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
