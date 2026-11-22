//! Finding duplicate photos, for the user to review. Nothing here deletes anything:
//! the review window shows each group with a suggested photo to keep, and only the
//! photos the user ticks go to the Trash (through `fileops::delete_photos`).
//!
//! Two kinds of search:
//! - **Exact copies**: the same bytes. Files are compared by size first, then by a quick
//!   fingerprint of their start and end, and only then read in full, so a big library on
//!   a network drive is mostly left alone. Two catalogue entries that are really the same
//!   file on disk (for example through a link) are never offered as copies of each other.
//! - **Look-alikes**: the same picture saved again, resized, re-compressed or lightly
//!   edited, or near-identical burst shots. Compared by a 64-bit "difference hash" made
//!   from the thumbnail Lumio already keeps, so this works with the drive unplugged.
//!   RAW + JPEG pairs (same name, same folder) are expected and never offered.
//!
//! Fingerprints are kept in `fingerprints` and reused until the file changes.

use crate::db::{self, Photo};
use crate::state::AppState;
use crate::thumbs;
use anyhow::{bail, Result};
use rayon::prelude::*;
use rusqlite::{params, params_from_iter, Connection};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tauri::Emitter;

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS fingerprints (
            photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
            sig TEXT NOT NULL,
            quick TEXT,
            full TEXT,
            dhash INTEGER,
            aspect REAL
        );",
    )?;
    Ok(())
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Options {
    /// "exact" or "similar".
    pub mode: String,
    /// Look-alikes: how many of the 64 fingerprint bits may differ (0 to 16).
    pub distance: u32,
    /// Only look in this folder and its subfolders.
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Member {
    pub photo: Photo,
    /// The photo Lumio suggests keeping (one per group).
    pub keep: bool,
    /// Look-alikes: how far its fingerprint is from the first photo's (0 = same).
    pub distance: u32,
    /// Its drive is connected, so it can be moved to the Trash.
    pub online: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Report {
    pub groups: Vec<Group>,
    /// Photos looked at.
    pub checked: usize,
    /// Exact: photos whose drive isn't connected. Look-alikes: photos without a thumbnail yet.
    pub skipped: usize,
    /// Space the extra copies take (exact copies only).
    pub extra_bytes: i64,
    pub cancelled: bool,
}

struct Candidate {
    id: i64,
    path: PathBuf,
    folder: String,
    filename: String,
    size: i64,
    sig: String,
    quick: Option<String>,
    full: Option<String>,
    dhash: Option<i64>,
    aspect: Option<f64>,
}

fn candidates(conn: &Connection, opts: &Options, photos_only: bool) -> Result<Vec<Candidate>> {
    let mut args: Vec<rusqlite::types::Value> = vec![];
    let mut wh = vec![
        "p.size > 0".to_string(),
        // A photo whose move is waiting isn't at its catalogue path yet.
        "NOT EXISTS (SELECT 1 FROM pending_moves m WHERE m.photo_id = p.id)".into(),
    ];
    if let Some(f) = opts.folder.as_deref().filter(|f| !f.is_empty()) {
        wh.push(db::folder_clause(f, true, &mut args).replace("folder", "p.folder"));
    }
    if photos_only {
        let video = crate::metadata::VIDEO_EXTS.iter().map(|e| format!("'{e}'")).collect::<Vec<_>>().join(",");
        wh.push(format!("lower(p.ext) NOT IN ({video}) AND p.thumb = 1"));
    }
    let sql = format!(
        "SELECT p.id, p.path, p.folder, p.filename, p.size, p.mtime, ifnull(p.orientation, 1),
                CASE WHEN f.sig = p.size || '-' || p.mtime || '-' || ifnull(p.orientation, 1) THEN 1 ELSE 0 END,
                f.quick, f.full, f.dhash, f.aspect
         FROM photos p LEFT JOIN fingerprints f ON f.photo_id = p.id
         WHERE {} ORDER BY coalesce(p.taken_at, datetime(p.mtime/1000, 'unixepoch')), p.id",
        wh.join(" AND ")
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(params_from_iter(args.iter()), |r| {
        let size: i64 = r.get(4)?;
        let mtime: i64 = r.get(5)?;
        let orient: i64 = r.get(6)?;
        let fresh = r.get::<_, i64>(7)? == 1;
        Ok(Candidate {
            id: r.get(0)?,
            path: PathBuf::from(r.get::<_, String>(1)?),
            folder: r.get(2)?,
            filename: r.get(3)?,
            size,
            sig: format!("{size}-{mtime}-{orient}"),
            quick: if fresh { r.get(8)? } else { None },
            full: if fresh { r.get(9)? } else { None },
            dhash: if fresh { r.get(10)? } else { None },
            aspect: if fresh { r.get(11)? } else { None },
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn save_fingerprints(conn: &Connection, c: &[Candidate]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut st = tx.prepare(
            "INSERT INTO fingerprints (photo_id, sig, quick, full, dhash, aspect) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(photo_id) DO UPDATE SET sig = excluded.sig, quick = excluded.quick, full = excluded.full,
                dhash = excluded.dhash, aspect = excluded.aspect",
        )?;
        for x in c {
            if x.quick.is_some() || x.full.is_some() || x.dhash.is_some() {
                // The photo may have left the catalogue while we were busy.
                let _ = st.execute(params![x.id, x.sig, x.quick, x.full, x.dhash, x.aspect]);
            }
        }
    }
    tx.commit()?;
    Ok(())
}

// ------------------------------------------------------------ exact copies

const QUICK_BYTES: u64 = 64 * 1024;

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// A fingerprint of the file's length, start and end: cheap, and enough to tell almost
/// all different files of the same size apart.
pub fn quick_hash(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut h = Sha1::new();
    h.update(len.to_le_bytes());
    let mut buf = vec![0u8; QUICK_BYTES.min(len) as usize];
    f.read_exact(&mut buf)?;
    h.update(&buf);
    if len > QUICK_BYTES {
        let tail = QUICK_BYTES.min(len - QUICK_BYTES);
        f.seek(SeekFrom::Start(len - tail))?;
        let mut buf = vec![0u8; tail as usize];
        f.read_exact(&mut buf)?;
        h.update(&buf);
    }
    Ok(hex(h.finalize()))
}

/// A fingerprint of the whole file.
pub fn full_hash(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if CANCEL.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(h.finalize()))
}

/// Indexes into `items` grouped by `key`, keeping only groups of two or more.
fn groups_by<K: std::hash::Hash + Eq>(idx: &[usize], key: impl Fn(usize) -> Option<K>) -> Vec<Vec<usize>> {
    let mut m: HashMap<K, Vec<usize>> = HashMap::new();
    let mut order = vec![];
    for &i in idx {
        if let Some(k) = key(i) {
            let e = m.entry(k).or_default();
            if e.is_empty() {
                order.push(i);
            }
            e.push(i);
        }
    }
    let mut out: Vec<Vec<usize>> = m.into_values().filter(|g| g.len() > 1).collect();
    // Keep the library's order (date taken), so results come out the same each time.
    out.sort_by_key(|g| g[0]);
    out
}

fn find_exact(c: &mut [Candidate], progress: &(dyn Fn(usize, usize, &str) + Sync)) -> (Vec<Vec<usize>>, usize) {
    // Only files that share their size with another can be copies.
    let all: Vec<usize> = (0..c.len()).collect();
    let same_size: Vec<usize> = groups_by(&all, |i| Some(c[i].size)).into_iter().flatten().collect();
    let total = same_size.len();
    let done = AtomicUsize::new(0);
    let missing = AtomicUsize::new(0);

    // Quick fingerprints, several files at a time.
    let quick: Vec<(usize, Option<String>)> = same_size
        .par_iter()
        .map(|&i| {
            let n = done.fetch_add(1, Ordering::Relaxed);
            if n.is_multiple_of(25) {
                progress(n, total, "Comparing files");
            }
            if CANCEL.load(Ordering::Relaxed) {
                return (i, None);
            }
            if !c[i].path.exists() {
                missing.fetch_add(1, Ordering::Relaxed);
                return (i, None);
            }
            if c[i].quick.is_some() {
                return (i, c[i].quick.clone());
            }
            (i, quick_hash(&c[i].path).ok())
        })
        .collect();
    let mut online = vec![];
    for (i, q) in quick {
        if q.is_some() {
            online.push(i);
        }
        c[i].quick = q;
    }
    // Same size and same quick fingerprint: read those in full.
    let maybe: Vec<usize> =
        groups_by(&online, |i| c[i].quick.clone().map(|q| (c[i].size, q))).into_iter().flatten().collect();
    let total = maybe.len();
    done.store(0, Ordering::Relaxed);
    let full: Vec<(usize, Option<String>)> = maybe
        .par_iter()
        .map(|&i| {
            let n = done.fetch_add(1, Ordering::Relaxed);
            progress(n, total, "Checking possible copies");
            if CANCEL.load(Ordering::Relaxed) {
                return (i, None);
            }
            if c[i].full.is_some() {
                return (i, c[i].full.clone());
            }
            (i, full_hash(&c[i].path).ok())
        })
        .collect();
    for (i, h) in full {
        c[i].full = h;
    }
    let groups = groups_by(&maybe, |i| c[i].full.clone().map(|h| (c[i].size, h)));
    // The same file reached by two paths (a link, or a folder added twice) is not a copy.
    let groups = groups.into_iter().filter_map(|g| distinct_files(g, |i| &c[i].path)).collect();
    (groups, missing.into_inner())
}

/// Leave out entries that are the same file on disk as an earlier one; `None` if fewer
/// than two different files are left.
fn distinct_files<'a>(g: Vec<usize>, path: impl Fn(usize) -> &'a Path) -> Option<Vec<usize>> {
    let mut seen = HashSet::new();
    let g: Vec<usize> = g
        .into_iter()
        .filter(|&i| seen.insert(std::fs::canonicalize(path(i)).unwrap_or_else(|_| path(i).to_path_buf())))
        .collect();
    (g.len() > 1).then_some(g)
}

// ------------------------------------------------------------ look-alikes

/// 64-bit difference hash: shrink to 9×8 grey pixels and note, for each pair of
/// neighbours, whether brightness rises from left to right. Robust to resizing,
/// re-compression and small colour or exposure changes.
pub fn dhash(img: &image::DynamicImage) -> u64 {
    let g = image::imageops::resize(&img.to_luma8(), 9, 8, image::imageops::FilterType::Triangle);
    let mut h = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            h <<= 1;
            if g.get_pixel(x, y)[0] < g.get_pixel(x + 1, y)[0] {
                h |= 1;
            }
        }
    }
    h
}

/// Plain or nearly plain pictures (a black frame, a clear sky) all look alike to the
/// hash, so they are left out of look-alike searches. Exact copies still find them.
fn too_plain(h: u64) -> bool {
    let n = h.count_ones();
    n <= 4 || n >= 60
}

fn stem(name: &str) -> String {
    Path::new(name).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Two photos the look-alike search may put together.
fn alike(a: &Candidate, b: &Candidate, max: u32) -> Option<u32> {
    let (ha, hb) = (a.dhash? as u64, b.dhash? as u64);
    let d = (ha ^ hb).count_ones();
    if d > max {
        return None;
    }
    // Different shapes (a crop, or portrait against landscape) are different pictures.
    if let (Some(x), Some(y)) = (a.aspect, b.aspect) {
        if (x / y).ln().abs() > 0.08 {
            return None;
        }
    }
    // RAW + JPEG of the same shot belong together; they are not duplicates.
    if a.folder == b.folder && stem(&a.filename) == stem(&b.filename) {
        return None;
    }
    Some(d)
}

/// For each usable photo (by position in `usable`), the later ones that look alike.
///
/// Fingerprints within `max` bits of each other agree exactly on at least one of `max + 1`
/// slices of the 64 bits (if every slice differed, at least `max + 1` bits would). So
/// only photos sharing a slice value are compared, which keeps large libraries near
/// linear. With wide tolerances the slices are too short to narrow much, and every
/// pair is compared instead (in parallel).
fn neighbours(c: &[Candidate], usable: &[usize], max: u32) -> Vec<Vec<(usize, u32)>> {
    let hash = |k: usize| c[usable[k]].dhash.unwrap_or(0) as u64;
    if max >= 8 {
        return usable
            .par_iter()
            .enumerate()
            .map(|(k, &i)| {
                if CANCEL.load(Ordering::Relaxed) {
                    return vec![];
                }
                usable[k + 1..].iter().filter_map(|&j| alike(&c[i], &c[j], max).map(|d| (j, d))).collect()
            })
            .collect();
    }
    let bands = bands(max);
    let slice = |h: u64, (start, width): (u32, u32)| (h >> start) & u64::MAX.checked_shr(64 - width).unwrap_or(0);
    let buckets: Vec<HashMap<u64, Vec<usize>>> = bands
        .iter()
        .map(|&b| {
            let mut m: HashMap<u64, Vec<usize>> = HashMap::new();
            for k in 0..usable.len() {
                m.entry(slice(hash(k), b)).or_default().push(k);
            }
            m
        })
        .collect();
    (0..usable.len())
        .into_par_iter()
        .map(|k| {
            if CANCEL.load(Ordering::Relaxed) {
                return vec![];
            }
            let h = hash(k);
            let mut out = vec![];
            for (n, &b) in bands.iter().enumerate() {
                for &l in buckets[n].get(&slice(h, b)).map(Vec::as_slice).unwrap_or(&[]) {
                    // Each pair once: from its first photo, in the first slice it shares.
                    if l <= k || bands[..n].iter().any(|&e| slice(h, e) == slice(hash(l), e)) {
                        continue;
                    }
                    if let Some(d) = alike(&c[usable[k]], &c[usable[l]], max) {
                        out.push((usable[l], d));
                    }
                }
            }
            out.sort_unstable();
            out
        })
        .collect()
}

/// `max + 1` slices covering the 64 bits, as (first bit, width).
fn bands(max: u32) -> Vec<(u32, u32)> {
    let n = max + 1;
    let mut out = vec![];
    let mut start = 0;
    for i in 0..n {
        let width = 64 / n + u32::from(i < 64 % n);
        out.push((start, width));
        start += width;
    }
    out
}

/// Group look-alikes. Each group is one photo and everything close to *it*, so a long
/// chain of slightly different photos never ends up as one huge group.
fn group_alike(c: &[Candidate], max: u32) -> Vec<Vec<(usize, u32)>> {
    let usable: Vec<usize> = (0..c.len()).filter(|&i| c[i].dhash.is_some_and(|h| !too_plain(h as u64))).collect();
    let near = neighbours(c, &usable, max);
    let mut adj: HashMap<usize, Vec<(usize, u32)>> = HashMap::new();
    for (k, list) in near.into_iter().enumerate() {
        let i = usable[k];
        for (j, d) in list {
            adj.entry(i).or_default().push((j, d));
            adj.entry(j).or_default().push((i, d));
        }
    }
    let mut taken = vec![false; c.len()];
    let mut out = vec![];
    for &i in &usable {
        if taken[i] {
            continue;
        }
        let Some(list) = adj.get(&i) else { continue };
        let mut g: Vec<(usize, u32)> = vec![(i, 0)];
        let mut more: Vec<(usize, u32)> = list.iter().copied().filter(|(j, _)| !taken[*j]).collect();
        if more.is_empty() {
            continue;
        }
        more.sort_by_key(|&(j, d)| (d, j));
        g.extend(more);
        for &(j, _) in &g {
            taken[j] = true;
        }
        out.push(g);
    }
    out
}

fn find_alike(
    state: &AppState,
    c: &mut [Candidate],
    max: u32,
    progress: &(dyn Fn(usize, usize, &str) + Sync),
) -> (Vec<Vec<(usize, u32)>>, usize) {
    let total = c.len();
    let done = AtomicUsize::new(0);
    let missing = AtomicUsize::new(0);
    let hashes: Vec<Option<(i64, f64)>> = c
        .par_iter()
        .map(|x| {
            let n = done.fetch_add(1, Ordering::Relaxed);
            if n.is_multiple_of(100) {
                progress(n, total, "Reading thumbnails");
            }
            if CANCEL.load(Ordering::Relaxed) {
                return None;
            }
            if let (Some(h), Some(a)) = (x.dhash, x.aspect) {
                return Some((h, a));
            }
            match image::open(thumbs::thumb_path(&state.thumbs_dir, x.id)) {
                Ok(img) if img.height() > 0 => Some((dhash(&img) as i64, img.width() as f64 / img.height() as f64)),
                _ => {
                    missing.fetch_add(1, Ordering::Relaxed);
                    None
                }
            }
        })
        .collect();
    for (x, h) in c.iter_mut().zip(hashes) {
        if let Some((h, a)) = h {
            x.dhash = Some(h);
            x.aspect = Some(a);
        }
    }
    progress(total, total, "Comparing photos");
    (group_alike(c, max), missing.into_inner())
}

// ------------------------------------------------------------ which one to keep

/// Names that say "I am a copy": "IMG_1 copy.jpg", "IMG_1 (2).jpg", "IMG_1-copy.jpg".
pub fn looks_like_copy(filename: &str) -> bool {
    let s = stem(filename);
    let s = s.trim();
    if s.ends_with("copy") || s.contains(" copy ") || s.ends_with("kopie") || s.ends_with("copie") {
        return true;
    }
    // " (2)" and similar.
    if let Some(open) = s.rfind(" (") {
        let inner = &s[open + 2..];
        if inner.ends_with(')') && inner[..inner.len() - 1].chars().all(|c| c.is_ascii_digit()) && inner.len() > 1 {
            return true;
        }
    }
    false
}

/// The photo to suggest keeping: the one with the most pixels, then the best rated and
/// best described, then one whose name doesn't look like a copy, then the oldest file
/// (more likely the original than an export), then the biggest file.
pub fn suggest_keep(photos: &[Photo]) -> usize {
    let score = |p: &Photo| {
        let pixels = p.width.unwrap_or(0) * p.height.unwrap_or(0);
        let described = p.label.is_some() as i64
            + !p.keywords.is_empty() as i64
            + p.title.as_deref().is_some_and(|t| !t.is_empty()) as i64
            + p.caption.as_deref().is_some_and(|t| !t.is_empty()) as i64
            + (p.lat.is_some()) as i64;
        (pixels, p.rating, described, !looks_like_copy(&p.filename), -p.mtime, p.size, -p.id)
    };
    (0..photos.len()).max_by_key(|&i| score(&photos[i])).unwrap_or(0)
}

// ------------------------------------------------------------ the search

pub fn find(state: &AppState, opts: &Options) -> Result<Report> {
    let similar = opts.mode == "similar";
    let max = opts.distance.min(16);
    let emit = |done: usize, total: usize, message: &str| {
        let _ = state
            .app
            .emit("duplicates-progress", serde_json::json!({ "done": done, "total": total, "message": message }));
    };
    emit(0, 0, "Looking through the catalogue");
    let mut c = candidates(&state.conn.lock(), opts, similar)?;
    let checked = c.len();
    let (groups, skipped): (Vec<Vec<(usize, u32)>>, usize) = if similar {
        find_alike(state, &mut c, max, &emit)
    } else {
        let (g, s) = find_exact(&mut c, &emit);
        (g.into_iter().map(|g| g.into_iter().map(|i| (i, 0)).collect()).collect(), s)
    };
    save_fingerprints(&state.conn.lock(), &c)?;
    let cancelled = CANCEL.load(Ordering::Relaxed);
    if cancelled {
        return Ok(Report { checked, cancelled, ..Default::default() });
    }
    let mut report = Report { checked, skipped, ..Default::default() };
    let conn = state.conn.lock();
    for g in groups {
        let ids: Vec<i64> = g.iter().map(|(i, _)| c[*i].id).collect();
        let found = db::get_photos(&conn, &ids)?;
        let by_id: HashMap<i64, Photo> = found.into_iter().map(|p| (p.id, p)).collect();
        let mut photos = vec![];
        let mut dist = vec![];
        for (i, d) in &g {
            if let Some(p) = by_id.get(&c[*i].id) {
                photos.push(p.clone());
                dist.push(*d);
            }
        }
        if photos.len() < 2 {
            continue;
        }
        let keep = suggest_keep(&photos);
        if !similar {
            report.extra_bytes += photos[0].size * (photos.len() as i64 - 1);
        }
        report.groups.push(Group {
            members: photos
                .into_iter()
                .zip(dist)
                .enumerate()
                .map(|(k, (p, distance))| Member { online: false, keep: k == keep, distance, photo: p })
                .collect(),
        });
    }
    // Asking the drives can be slow (network drives), so it happens without the lock.
    drop(conn);
    for m in report.groups.iter_mut().flat_map(|g| g.members.iter_mut()) {
        m.online = Path::new(&m.photo.path).exists();
    }
    Ok(report)
}

pub mod cmd {
    use super::*;
    use std::sync::Arc;
    use tauri::State;

    /// Look for duplicates. Only finds them: nothing is moved or deleted.
    #[tauri::command]
    pub async fn find_duplicates(state: State<'_, Arc<AppState>>, opts: Options) -> Result<Report, String> {
        if RUNNING.swap(true, Ordering::SeqCst) {
            return Err("Lumio is already looking for duplicates.".into());
        }
        CANCEL.store(false, Ordering::SeqCst);
        let st = state.inner().clone();
        let r = tauri::async_runtime::spawn_blocking(move || find(&st, &opts)).await;
        RUNNING.store(false, Ordering::SeqCst);
        r.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
    }

    #[tauri::command]
    pub fn cancel_duplicates() {
        CANCEL.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    fn cand(id: i64, folder: &str, name: &str, dhash: u64, aspect: f64) -> Candidate {
        Candidate {
            id,
            path: PathBuf::from(format!("{folder}/{name}")),
            folder: folder.into(),
            filename: name.into(),
            size: 1,
            sig: String::new(),
            quick: None,
            full: None,
            dhash: Some(dhash as i64),
            aspect: Some(aspect),
        }
    }

    #[test]
    fn slicing_finds_exactly_what_comparing_every_pair_finds() {
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        // Clusters of near copies (a few bits flipped) among unrelated pictures.
        let mut c = vec![];
        for g in 0..150 {
            let base = next();
            for m in 0..4 {
                let mut h = base;
                for _ in 0..(m * 2) {
                    h ^= 1 << (next() % 64);
                }
                c.push(cand(c.len() as i64, &format!("/f{g}"), &format!("{m}.jpg"), h, 1.5));
            }
        }
        let usable: Vec<usize> = (0..c.len()).filter(|&i| c[i].dhash.is_some_and(|h| !too_plain(h as u64))).collect();
        for max in 0..8 {
            let brute: Vec<Vec<(usize, u32)>> = usable
                .iter()
                .enumerate()
                .map(|(k, &i)| {
                    usable[k + 1..].iter().filter_map(|&j| alike(&c[i], &c[j], max).map(|d| (j, d))).collect()
                })
                .collect();
            assert_eq!(neighbours(&c, &usable, max), brute, "max {max}");
        }
        let total: u32 = bands(5).iter().map(|b| b.1).sum();
        assert_eq!(total, 64);
    }

    fn picture(w: u32, h: u32, seed: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            let v = ((x * 7 + y * 3 + seed * 50) % 256) as u8;
            let s = (((x / 9) ^ (y / 7)) * 37 + seed * 91) as u8;
            Rgb([v, s, v.wrapping_add(s)])
        }))
    }

    #[test]
    fn dhash_survives_resizing_but_not_a_different_picture() {
        let a = picture(480, 320, 1);
        let small = a.resize_exact(240, 160, image::imageops::FilterType::Lanczos3);
        let other = picture(480, 320, 4);
        let (ha, hs, ho) = (dhash(&a), dhash(&small), dhash(&other));
        assert!((ha ^ hs).count_ones() <= 4, "resized copy differs by {}", (ha ^ hs).count_ones());
        assert!((ha ^ ho).count_ones() > 16, "different picture only differs by {}", (ha ^ ho).count_ones());
        // A plain picture is "too plain" to compare.
        let plain = DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 80, Rgb([20, 20, 20])));
        assert!(too_plain(dhash(&plain)));
    }

    #[test]
    fn groups_do_not_chain_and_skip_raw_jpeg_pairs() {
        let base: u64 = 0x5a5a_5a5a_0f0f_3c3c;
        let c = vec![
            cand(1, "/a", "IMG_1.CR3", base, 1.5),
            cand(2, "/a", "IMG_1.JPG", base, 1.5), // its RAW + JPEG partner: not a duplicate
            cand(3, "/b", "IMG_1 export.jpg", base ^ 0b11, 1.5), // 2 bits away: a look-alike
            cand(4, "/c", "far.jpg", base ^ 0b1111_1100, 1.5), // 6 from 1, 4 from 3
            cand(5, "/d", "portrait.jpg", base, 0.66), // same hash, other shape
        ];
        let g = group_alike(&c, 4);
        assert_eq!(g.len(), 1);
        let ids: Vec<i64> = g[0].iter().map(|(i, _)| c[*i].id).collect();
        assert_eq!(ids, [1, 3]);
        assert_eq!(g[0][1].1, 2);
        // With a looser limit, 4 joins the first photo's group directly, not through 3.
        let g = group_alike(&c, 6);
        let ids: Vec<i64> = g[0].iter().map(|(i, _)| c[*i].id).collect();
        assert_eq!(ids, [1, 3, 4]);
    }

    #[test]
    fn exact_hashes() {
        let dir = std::env::temp_dir().join(format!("lumio-dupes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let mut changed_middle = big.clone();
        changed_middle[150_000] ^= 1;
        std::fs::write(dir.join("a.jpg"), &big).unwrap();
        std::fs::write(dir.join("b.jpg"), &big).unwrap();
        std::fs::write(dir.join("c.jpg"), &changed_middle).unwrap();
        std::fs::write(dir.join("tiny.jpg"), b"abc").unwrap();
        let q = |n: &str| quick_hash(&dir.join(n)).unwrap();
        let f = |n: &str| full_hash(&dir.join(n)).unwrap();
        assert_eq!(q("a.jpg"), q("b.jpg"));
        // Same start and end, so the quick fingerprint can't tell; the full one can.
        assert_eq!(q("a.jpg"), q("c.jpg"));
        assert_eq!(f("a.jpg"), f("b.jpg"));
        assert_ne!(f("a.jpg"), f("c.jpg"));
        assert_eq!(q("tiny.jpg").len(), 40);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_names() {
        assert!(looks_like_copy("IMG_0001 copy.jpg"));
        assert!(looks_like_copy("IMG_0001 (2).jpg"));
        assert!(looks_like_copy("IMG_0001-copy.JPG"));
        assert!(!looks_like_copy("IMG_0001.jpg"));
        assert!(!looks_like_copy("Party (Rome).jpg"));
        assert!(!looks_like_copy("Copyright test.jpg"));
    }

    #[test]
    fn keeps_the_best_copy() {
        let p = |id: i64, name: &str, w: i64, rating: i64, mtime: i64| Photo {
            id,
            filename: name.into(),
            width: Some(w),
            height: Some(w * 2 / 3),
            rating,
            mtime,
            size: 100,
            ..Default::default()
        };
        // Largest wins.
        assert_eq!(suggest_keep(&[p(1, "a.jpg", 2000, 5, 1), p(2, "b.jpg", 6000, 0, 2)]), 1);
        // Same size: the rated one.
        assert_eq!(suggest_keep(&[p(1, "a.jpg", 6000, 0, 1), p(2, "b.jpg", 6000, 3, 2)]), 1);
        // All else equal: not the one called "copy", then the older file.
        assert_eq!(suggest_keep(&[p(1, "a copy.jpg", 6000, 0, 1), p(2, "a.jpg", 6000, 0, 2)]), 1);
        assert_eq!(suggest_keep(&[p(1, "a.jpg", 6000, 0, 9), p(2, "b.jpg", 6000, 0, 2)]), 1);
    }

    #[test]
    fn same_file_through_two_paths_is_not_a_copy() {
        let dir = std::env::temp_dir().join(format!("lumio-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("real.jpg"), vec![7u8; 1000]).unwrap();
        std::fs::write(dir.join("copy.jpg"), vec![7u8; 1000]).unwrap();
        let paths = [dir.join("real.jpg"), dir.join("sub").join("..").join("real.jpg"), dir.join("copy.jpg")];
        assert_eq!(distinct_files(vec![0, 1, 2], |i| &paths[i]), Some(vec![0, 2]));
        assert_eq!(distinct_files(vec![0, 1], |i| &paths[i]), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exact_search_end_to_end() {
        let dir = std::env::temp_dir().join(format!("lumio-exact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
        let mut middle = data.clone();
        middle[100_000] ^= 1;
        let files: [(&str, Option<&[u8]>); 6] = [
            ("one.jpg", Some(&data)),
            ("two.jpg", Some(&data)),
            ("almost.jpg", Some(&middle)),
            ("short.jpg", Some(&data[..1000])),
            ("three.jpg", Some(&data)),
            ("gone.jpg", None), // its drive is away
        ];
        let mut c: Vec<Candidate> = files
            .iter()
            .enumerate()
            .map(|(i, (name, bytes))| {
                if let Some(b) = bytes {
                    std::fs::write(dir.join(name), b).unwrap();
                }
                let mut x = cand(i as i64, &dir.to_string_lossy(), name, 0, 1.0);
                x.path = dir.join(name);
                x.size = bytes.map_or(200_000, |b| b.len() as i64);
                x
            })
            .collect();
        let (groups, missing) = find_exact(&mut c, &|_, _, _| {});
        assert_eq!(missing, 1);
        assert_eq!(groups, vec![vec![0, 1, 4]]);
        // Fingerprints are remembered for next time.
        assert!(c[0].full.is_some() && c[2].full.is_some() && c[3].quick.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
