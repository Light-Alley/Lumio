//! Renaming photos, one at a time or many at once, while keeping the catalogue,
//! thumbnails, ratings, groups and XMP sidecars attached to each photo.
//!
//! Batch renames are planned first (`plan`), shown to the person as a preview, and then
//! carried out with `rename_many`, which also powers Undo. Names that swap places inside a
//! batch (A -> B while B -> A) are handled by parking files under a hidden temporary name first.

use crate::db::{self, Photo};
use crate::fileops::{check_name, sidecar_target};
use crate::indexer::{self, find_sidecar};
use crate::metadata;
use crate::state::AppState;
use anyhow::{anyhow, bail, Context, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Case {
    #[default]
    Keep,
    Lower,
    Upper,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Build each name from a pattern such as "{date} Holiday {n}".
    #[default]
    Pattern,
    /// Replace some text in the current names.
    Replace,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct BatchOptions {
    pub mode: Mode,
    pub pattern: String,
    pub start: i64,
    pub digits: u32,
    pub find: String,
    pub replace: String,
    pub match_case: bool,
    pub case: Case,
    pub ext_case: Case,
    /// Give a RAW and its JPEG (same name, same folder) the same new name and number.
    pub keep_pairs: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedName {
    pub id: i64,
    pub old: String,
    pub new: String,
    /// Why this one can't be renamed as planned, if it can't.
    pub problem: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct RenameReport {
    pub done: usize,
    pub failed: Vec<(String, String)>,
    /// (id, previous name) for everything renamed, so the app can offer Undo.
    pub undo: Vec<(i64, String)>,
}

/// Split "IMG_0001.CR2" into ("IMG_0001", ".CR2"); a name without a dot has no extension.
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

fn apply_case(s: &str, c: Case) -> String {
    match c {
        Case::Keep => s.to_string(),
        Case::Lower => s.to_lowercase(),
        Case::Upper => s.to_uppercase(),
    }
}

/// Make text taken from metadata safe to use in a file name.
fn clean(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '-' } else { c })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The date and time a photo was taken, or failing that when the file was last changed
/// (the catalogue keeps that in milliseconds).
fn when(p: &Photo) -> Option<chrono::NaiveDateTime> {
    if let Some(t) = &p.taken_at {
        if let Ok(d) = chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S") {
            return Some(d);
        }
    }
    chrono::DateTime::from_timestamp_millis(p.mtime).map(|d| d.with_timezone(&chrono::Local).naive_local())
}

pub const TOKENS: &[&str] = &["name", "n", "date", "time", "year", "month", "day", "folder", "camera", "title"];

fn expand(pattern: &str, p: &Photo, stem: &str, n: i64, digits: u32) -> Result<String> {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let j = after.find('}').ok_or_else(|| anyhow!("A \"{{\" in the pattern has no closing \"}}\"."))?;
        let token = after[..j].trim().to_lowercase();
        let d = when(p);
        let fmt = |f: &str| d.map(|d| d.format(f).to_string()).unwrap_or_default();
        let v = match token.as_str() {
            "name" => stem.to_string(),
            "n" | "#" => {
                let w = digits.clamp(1, 9) as usize;
                if n < 0 { format!("-{:0w$}", -n) } else { format!("{n:0w$}") }
            }
            "date" => fmt("%Y-%m-%d"),
            "time" => fmt("%H%M%S"),
            "year" => fmt("%Y"),
            "month" => fmt("%m"),
            "day" => fmt("%d"),
            "folder" => Path::new(&p.folder).file_name().map(|s| clean(&s.to_string_lossy())).unwrap_or_default(),
            "camera" => clean(p.model.as_deref().unwrap_or("")),
            "title" => clean(p.title.as_deref().unwrap_or("")),
            other => bail!("\"{{{other}}}\" is not something Lumio can put in a name. Use one of: {}.", TOKENS.iter().map(|t| format!("{{{t}}}")).collect::<Vec<_>>().join(", ")),
        };
        out.push_str(&v);
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn replace_text(stem: &str, find: &str, with: &str, match_case: bool) -> String {
    if find.is_empty() {
        return stem.to_string();
    }
    if match_case {
        return stem.replace(find, with);
    }
    let lower = stem.to_lowercase();
    let needle = find.to_lowercase();
    // Lower-casing can change byte lengths for some scripts; fall back to exact match then.
    if lower.len() != stem.len() || needle.len() != find.len() {
        return stem.replace(find, with);
    }
    let mut out = String::new();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find(&needle) {
        out.push_str(&stem[pos..pos + i]);
        out.push_str(with);
        pos += i + needle.len();
    }
    out.push_str(&stem[pos..]);
    out
}

/// Work out the new names for `photos` (in the order given) without touching any files.
pub fn plan(photos: &[Photo], o: &BatchOptions) -> Result<Vec<PlannedName>> {
    if o.mode == Mode::Pattern && o.pattern.trim().is_empty() {
        bail!("Type a pattern for the new names.");
    }
    let mut out = Vec::with_capacity(photos.len());
    let mut n = o.start;
    let mut pair_numbers: HashMap<(String, String), i64> = HashMap::new();
    for p in photos {
        let (stem, ext) = split_name(&p.filename);
        let num = if o.keep_pairs {
            let key = (p.folder.clone(), stem.to_lowercase());
            *pair_numbers.entry(key).or_insert_with(|| {
                let v = n;
                n += 1;
                v
            })
        } else {
            let v = n;
            n += 1;
            v
        };
        let new_stem = match o.mode {
            Mode::Pattern => expand(&o.pattern, p, stem, num, o.digits)?,
            Mode::Replace => replace_text(stem, &o.find, &o.replace, o.match_case),
        };
        let new_stem = apply_case(new_stem.trim(), o.case);
        let new = format!("{new_stem}{}", apply_case(ext, o.ext_case));
        out.push(PlannedName { id: p.id, old: p.filename.clone(), new, problem: None });
    }
    // Problems we can see before touching anything.
    let by_id: HashMap<i64, &Photo> = photos.iter().map(|p| (p.id, p)).collect();
    let leaving: HashSet<String> = photos.iter().map(|p| p.path.to_lowercase()).collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for i in 0..out.len() {
        let p = by_id[&out[i].id];
        let new = out[i].new.clone();
        if let Err(e) = check_new_name(&new) {
            out[i].problem = Some(e.to_string());
            continue;
        }
        let dst = Path::new(&p.folder).join(&new);
        let key = dst.to_string_lossy().to_lowercase();
        if let Some(&first) = seen.get(&key) {
            out[i].problem = Some(format!("Same new name as {}", out[first].old));
            continue;
        }
        seen.insert(key.clone(), i);
        if new != p.filename && !leaving.contains(&key) && dst.exists() {
            out[i].problem = Some("A file with this name is already in the folder".into());
        }
    }
    Ok(out)
}

/// Rules for a new photo name on top of the general name check.
fn check_new_name(name: &str) -> Result<()> {
    check_name(name)?;
    let (stem, _) = split_name(name);
    if stem.trim().is_empty() {
        bail!("The name is empty.");
    }
    if name.starts_with('.') {
        bail!("Names starting with a dot are hidden; choose another name.");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        bail!("Names can't end with a dot or a space (Windows doesn't allow it).");
    }
    if name.chars().any(|c| c.is_control()) {
        bail!("The name contains characters that can't be used in a file name.");
    }
    Ok(())
}

/// The name to use when a person types a new name for one photo. Keeps the photo's
/// extension unless they typed the same extension (perhaps in another case), so
/// "Beach.v2" for "IMG_1.jpg" becomes "Beach.v2.jpg" rather than an unreadable file.
pub fn single_name(old: &str, typed: &str) -> String {
    let typed = typed.trim();
    let (_, old_ext) = split_name(old);
    if old_ext.is_empty() {
        return typed.to_string();
    }
    let (_, new_ext) = split_name(typed);
    if new_ext.eq_ignore_ascii_case(old_ext) {
        typed.to_string()
    } else {
        format!("{typed}{old_ext}")
    }
}

struct Item {
    photo: Photo,
    src: PathBuf,
    dst: PathBuf,
    name: String,
    /// Sidecar found before anything moved, and whether to copy it (it belongs to another file too).
    side: Option<(PathBuf, bool)>,
    /// Hidden temporary name while the file waits for its final name.
    parked: Option<PathBuf>,
    parked_side: Option<PathBuf>,
}

/// Is a "name.xmp" sidecar also the sidecar of a file that is staying put?
fn sidecar_shared(src: &Path, side: &Path, moving: &HashSet<PathBuf>) -> bool {
    let side_name = side.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    let full = src.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    if side_name == format!("{full}.xmp") {
        return false; // "IMG_1.CR2.xmp" belongs to that one file.
    }
    let stem = src.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
    let Some(dir) = src.parent() else { return false };
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().map(|e| e.path()).any(|p| {
        p != src
            && !moving.contains(&p)
            && p.file_stem().map(|s| s.to_string_lossy().to_lowercase()) == Some(stem.clone())
            && metadata::is_supported(&indexer::ext_of(&p))
    })
}

fn same_place(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// Rename each photo in `items` (id, new file name) in its own folder.
pub fn rename_many(state: &AppState, items: &[(i64, String)]) -> Result<RenameReport> {
    rename_in(&state.conn, items)
}

fn rename_in(conn: &Mutex<Connection>, items: &[(i64, String)]) -> Result<RenameReport> {
    let mut rep = RenameReport::default();
    let photos = { db::get_photos(&conn.lock(), &items.iter().map(|(id, _)| *id).collect::<Vec<_>>())? };
    let by_id: HashMap<i64, Photo> = photos.into_iter().map(|p| (p.id, p)).collect();

    // 1. Check everything first.
    let mut work: Vec<Item> = vec![];
    let mut targets: HashSet<String> = HashSet::new();
    for (id, name) in items {
        let Some(p) = by_id.get(id).cloned() else { continue };
        let name = name.trim().to_string();
        if name == p.filename {
            continue;
        }
        let src = PathBuf::from(&p.path);
        let dst = src.with_file_name(&name);
        let problem = if let Err(e) = check_new_name(&name) {
            Some(e.to_string())
        } else if !src.exists() {
            Some("the file is not available (is its drive connected?)".into())
        } else if !targets.insert(dst.to_string_lossy().to_lowercase()) {
            Some(format!("another photo in this batch is also being named \"{name}\""))
        } else {
            None
        };
        if let Some(why) = problem {
            rep.failed.push((p.filename.clone(), why));
            continue;
        }
        work.push(Item { photo: p, src, dst, name, side: None, parked: None, parked_side: None });
    }
    let sources: HashSet<PathBuf> = work.iter().map(|w| w.src.clone()).collect();
    let sources_lc: HashSet<String> = sources.iter().map(|s| s.to_string_lossy().to_lowercase()).collect();
    work.retain(|w| {
        let freed_by_batch = sources_lc.contains(&w.dst.to_string_lossy().to_lowercase());
        if w.dst.exists() && !freed_by_batch {
            rep.failed.push((w.photo.filename.clone(), format!("a file named \"{}\" is already in the folder", w.name)));
            false
        } else {
            true
        }
    });
    for w in &mut work {
        w.side = find_sidecar(&w.src).map(|s| {
            let shared = sidecar_shared(&w.src, &s, &sources);
            (s, shared)
        });
    }

    // 2. Move aside, under a hidden temporary name, every file whose current name another
    //    photo in the batch is about to take (including a file that only changes case,
    //    which many drives treat as the same name).
    let pid = std::process::id();
    let wanted: Vec<PathBuf> = work.iter().map(|w| w.dst.clone()).collect();
    for (i, w) in work.iter_mut().enumerate() {
        if !wanted.iter().any(|d| same_place(d, &w.src)) {
            continue;
        }
        let dir = w.src.parent().unwrap_or(Path::new("."));
        let tmp = dir.join(format!(".lumio-rename-{pid}-{i}.tmp"));
        if let Err(e) = std::fs::rename(&w.src, &tmp) {
            rep.failed.push((w.photo.filename.clone(), format!("could not rename: {e}")));
            w.dst = PathBuf::new();
            continue;
        }
        w.parked = Some(tmp);
        if let Some((s, false)) = &w.side {
            let ts = dir.join(format!(".lumio-rename-{pid}-{i}.xmp.tmp"));
            if std::fs::rename(s, &ts).is_ok() {
                w.parked_side = Some(ts);
            }
        }
    }

    // 3. Give every file its final name.
    let mut touched: Vec<PathBuf> = vec![];
    let mut renamed: Vec<(i64, PathBuf, String)> = vec![];
    for w in work.iter().filter(|w| !w.dst.as_os_str().is_empty()) {
        let from = w.parked.clone().unwrap_or_else(|| w.src.clone());
        let res: Result<()> = (|| {
            if w.dst.exists() {
                bail!("a file named \"{}\" is already in the folder", w.name);
            }
            std::fs::rename(&from, &w.dst).with_context(|| format!("could not rename {}", w.photo.filename))?;
            Ok(())
        })();
        if let Err(e) = res {
            // Put a parked file back under its old name.
            if let Some(t) = &w.parked {
                let _ = std::fs::rename(t, &w.src);
                if let Some(ts) = &w.parked_side {
                    if let Some((s, _)) = &w.side {
                        let _ = std::fs::rename(ts, s);
                    }
                }
            }
            rep.failed.push((w.photo.filename.clone(), format!("{e:#}")));
            continue;
        }
        if let Some((s, shared)) = &w.side {
            let target = sidecar_target(&w.dst, s);
            let from_side = w.parked_side.clone().unwrap_or_else(|| s.clone());
            let r = if *shared { std::fs::copy(s, &target).map(|_| ()) } else { std::fs::rename(&from_side, &target) };
            if let Err(e) = r {
                log::warn!("renamed {} but not its sidecar: {e}", w.dst.display());
            }
        }
        renamed.push((w.photo.id, w.dst.clone(), w.name.clone()));
        rep.done += 1;
        rep.undo.push((w.photo.id, w.photo.filename.clone()));
        if let Some(par) = w.dst.parent() {
            if !touched.iter().any(|t| t == par) {
                touched.push(par.to_path_buf());
            }
        }
    }
    // 4. Update the catalogue in one go. Paths are unique, so when names swap places each
    //    row first gets a placeholder path.
    {
        let conn = conn.lock();
        let tx = conn.unchecked_transaction()?;
        for (id, _, _) in &renamed {
            tx.execute("UPDATE photos SET path = '//lumio-renaming/' || id WHERE id = ?1", [id])?;
        }
        for (id, dst, name) in &renamed {
            tx.execute(
                "UPDATE photos SET path = ?1, filename = ?2, ext = ?3 WHERE id = ?4",
                params![dst.to_string_lossy(), name, indexer::ext_of(dst), id],
            )?;
        }
        tx.commit()?;
    }
    crate::notify::changed(&touched);
    Ok(rep)
}

/// Rename one photo to the name a person typed.
pub fn rename_one(state: &AppState, id: i64, typed: &str) -> Result<RenameReport> {
    let p = { db::get_photo(&state.conn.lock(), id)?.ok_or_else(|| anyhow!("photo not found"))? };
    let name = single_name(&p.filename, typed);
    check_new_name(&name)?;
    let rep = rename_many(state, &[(id, name)])?;
    if let Some((_, why)) = rep.failed.first() {
        bail!("{}", capitalise(why));
    }
    Ok(rep)
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(id: i64, folder: &str, name: &str) -> Photo {
        Photo {
            id,
            path: format!("{folder}/{name}"),
            folder: folder.into(),
            filename: name.into(),
            taken_at: Some("2024-07-14T09:05:30".into()),
            model: Some("Canon EOS R5".into()),
            mtime: 1_700_000_000_000,
            ..Default::default()
        }
    }

    fn opts(pattern: &str) -> BatchOptions {
        BatchOptions { pattern: pattern.into(), start: 1, digits: 3, ..Default::default() }
    }

    #[test]
    fn pattern_tokens() {
        let ps = vec![photo(1, "/x/Iceland", "IMG_0001.JPG"), photo(2, "/x/Iceland", "IMG_0002.JPG")];
        let r = plan(&ps, &opts("{date} {folder} {n}")).unwrap();
        assert_eq!(r[0].new, "2024-07-14 Iceland 001.JPG");
        assert_eq!(r[1].new, "2024-07-14 Iceland 002.JPG");
        let r = plan(&ps, &opts("{year}{month}{day}_{time}_{camera}_{name}")).unwrap();
        assert_eq!(r[0].new, "20240714_090530_Canon EOS R5_IMG_0001.JPG");
        assert!(plan(&ps, &opts("{nope}")).is_err());
        // Without a date taken, the file's date is used.
        let mut undated = ps[0].clone();
        undated.taken_at = None;
        assert!(plan(&[undated], &opts("{year}")).unwrap()[0].new.starts_with("2023"));
        assert!(plan(&ps, &opts("{date")).is_err());
    }

    #[test]
    fn raw_and_jpeg_share_a_number() {
        let ps = vec![photo(1, "/x", "IMG_1.CR2"), photo(2, "/x", "IMG_1.JPG"), photo(3, "/x", "IMG_2.CR2")];
        let mut o = opts("Trip {n}");
        o.keep_pairs = true;
        let r = plan(&ps, &o).unwrap();
        assert_eq!([&r[0].new, &r[1].new, &r[2].new], ["Trip 001.CR2", "Trip 001.JPG", "Trip 002.CR2"]);
        o.keep_pairs = false;
        let r = plan(&ps, &o).unwrap();
        assert_eq!(r[1].new, "Trip 002.JPG");
    }

    #[test]
    fn duplicates_and_bad_names_are_flagged() {
        let ps = vec![photo(1, "/x", "a.jpg"), photo(2, "/x", "b.jpg")];
        let r = plan(&ps, &opts("Same")).unwrap();
        assert!(r[0].problem.is_none());
        assert!(r[1].problem.as_deref().unwrap().contains("Same new name"));
        let r = plan(&ps, &opts("bad/name {n}")).unwrap();
        assert!(r[0].problem.is_some());
    }

    #[test]
    fn replace_and_case() {
        let ps = vec![photo(1, "/x", "DSC_0042.NEF")];
        let o = BatchOptions { mode: Mode::Replace, find: "dsc_".into(), replace: "Kenya ".into(), ext_case: Case::Lower, ..Default::default() };
        assert_eq!(plan(&ps, &o).unwrap()[0].new, "Kenya 0042.nef");
        let o = BatchOptions { match_case: true, ..o };
        assert_eq!(plan(&ps, &o).unwrap()[0].new, "DSC_0042.nef");
    }

    #[test]
    fn single_name_keeps_extension() {
        assert_eq!(single_name("IMG_1.jpg", "Beach"), "Beach.jpg");
        assert_eq!(single_name("IMG_1.jpg", "Beach.v2"), "Beach.v2.jpg");
        assert_eq!(single_name("IMG_1.jpg", "Beach.JPG"), "Beach.JPG");
        assert_eq!(single_name("README", "notes"), "notes");
    }

    // ---- renaming real files -------------------------------------------------------

    fn setup(files: &[&str]) -> (PathBuf, Mutex<Connection>, Vec<i64>) {
        let dir = std::env::temp_dir().join(format!("lumio-rename-test-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn.execute("INSERT INTO roots (id, path, name, added_at) VALUES (1, ?1, 'r', 0)", [dir.to_string_lossy()]).unwrap();
        let mut ids = vec![];
        for f in files {
            let p = dir.join(f);
            std::fs::write(&p, f.as_bytes()).unwrap();
            if f.ends_with(".xmp") {
                continue;
            }
            conn.execute(
                "INSERT INTO photos (root_id, path, folder, filename, ext, rating) VALUES (1, ?1, ?2, ?3, ?4, 3)",
                params![p.to_string_lossy(), dir.to_string_lossy(), f, indexer::ext_of(&p)],
            )
            .unwrap();
            ids.push(conn.last_insert_rowid());
        }
        (dir, Mutex::new(conn), ids)
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        v.sort();
        v
    }

    fn content(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    fn catalogue(conn: &Mutex<Connection>, id: i64) -> (String, String, i64) {
        conn.lock()
            .query_row("SELECT path, filename, rating FROM photos WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
    }

    #[test]
    fn renames_file_sidecar_and_catalogue() {
        let (dir, conn, ids) = setup(&["IMG_1.CR2", "IMG_1.CR2.xmp", "IMG_2.JPG", "IMG_2.xmp"]);
        let r = rename_in(&conn, &[(ids[0], "Beach.CR2".into()), (ids[1], "Dunes.JPG".into())]).unwrap();
        assert_eq!(r.done, 2, "{:?}", r.failed);
        assert_eq!(names(&dir), ["Beach.CR2", "Beach.CR2.xmp", "Dunes.JPG", "Dunes.xmp"]);
        assert_eq!(content(&dir, "Beach.CR2"), "IMG_1.CR2");
        assert_eq!(content(&dir, "Dunes.xmp"), "IMG_2.xmp");
        let (path, name, rating) = catalogue(&conn, ids[0]);
        assert_eq!((name.as_str(), rating), ("Beach.CR2", 3));
        assert_eq!(PathBuf::from(path), dir.join("Beach.CR2"));
        // Undo puts everything back.
        let u = rename_in(&conn, &r.undo).unwrap();
        assert_eq!(u.done, 2);
        assert_eq!(names(&dir), ["IMG_1.CR2", "IMG_1.CR2.xmp", "IMG_2.JPG", "IMG_2.xmp"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn swapping_names_and_changing_case() {
        let (dir, conn, ids) = setup(&["a.jpg", "b.jpg", "c.jpg"]);
        let r = rename_in(&conn, &[(ids[0], "b.jpg".into()), (ids[1], "a.jpg".into()), (ids[2], "C.JPG".into())]).unwrap();
        assert_eq!(r.done, 3, "{:?}", r.failed);
        assert_eq!(names(&dir), ["C.JPG", "a.jpg", "b.jpg"]);
        assert_eq!(content(&dir, "b.jpg"), "a.jpg");
        assert_eq!(content(&dir, "a.jpg"), "b.jpg");
        assert_eq!(catalogue(&conn, ids[0]).1, "b.jpg");
        assert_eq!(catalogue(&conn, ids[1]).1, "a.jpg");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn never_overwrites_other_files() {
        let (dir, conn, ids) = setup(&["a.jpg", "b.jpg", "keep.jpg"]);
        let r = rename_in(&conn, &[(ids[0], "keep.jpg".into()), (ids[1], "x.jpg".into())]).unwrap();
        assert_eq!(r.done, 1);
        assert_eq!(r.failed.len(), 1);
        assert_eq!(content(&dir, "keep.jpg"), "keep.jpg");
        assert_eq!(names(&dir), ["a.jpg", "keep.jpg", "x.jpg"]);
        // Two photos asking for the same name: only the first gets it.
        let r = rename_in(&conn, &[(ids[0], "same.jpg".into()), (ids[1], "same.jpg".into())]).unwrap();
        assert_eq!((r.done, r.failed.len()), (1, 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn shared_sidecar_stays_for_the_file_left_behind() {
        // IMG_1.xmp is the RAW file's sidecar (Lightroom naming); renaming only the JPEG
        // leaves it with the RAW file.
        let (dir, conn, ids) = setup(&["IMG_1.CR2", "IMG_1.JPG", "IMG_1.xmp"]);
        let r = rename_in(&conn, &[(ids[1], "Fox.JPG".into())]).unwrap();
        assert_eq!(r.done, 1);
        assert_eq!(names(&dir), ["Fox.JPG", "IMG_1.CR2", "IMG_1.xmp"]);
        // Renaming the pair together moves it.
        let (dir2, conn2, ids2) = setup(&["IMG_1.CR2", "IMG_1.JPG", "IMG_1.xmp"]);
        let r = rename_in(&conn2, &[(ids2[0], "Fox.CR2".into()), (ids2[1], "Fox.JPG".into())]).unwrap();
        assert_eq!(r.done, 2);
        assert_eq!(names(&dir2), ["Fox.CR2", "Fox.JPG", "Fox.xmp"]);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&dir2).ok();
    }
}
