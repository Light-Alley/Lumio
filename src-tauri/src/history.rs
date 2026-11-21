//! The undo history: every change Lumio makes to files (moves, copies, renames, new
//! folders, things sent to the Trash), newest first, kept in the catalogue so it is still
//! there after a restart.
//!
//! Undo goes back in order: undoing an older change first undoes everything after it, so
//! each step finds the files where it left them. A step checks each photo is still where
//! that change put it and leaves alone anything that has been moved or renamed since.
//! Undo never overwrites a file and never deletes an original; the only files it removes
//! are copies Lumio made, and those go to the Trash.

use crate::db;
use crate::fileops::{self, Host, Moved, OpReport};
use crate::indexer::Job;
use crate::rename::{self, RenameReport};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How many changes are remembered.
const KEEP: i64 = 300;

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at INTEGER NOT NULL,
            label TEXT NOT NULL,
            change TEXT NOT NULL,
            undone_at INTEGER,
            note TEXT
        );",
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    Move { items: Vec<Moved> },
    /// (copy, original) for every file a copy made.
    Copy { files: Vec<(PathBuf, PathBuf)> },
    /// (photo id, old name, new name).
    Rename { items: Vec<(i64, String, String)> },
    FolderRename { from: PathBuf, to: PathBuf },
    FolderMove { from: PathBuf, to: PathBuf },
    FolderCopy { to: PathBuf },
    FolderCreate { path: PathBuf },
    /// Kept so the history is complete; restoring is done from the system Trash.
    Trash { count: usize, permanent: bool },
}

impl Change {
    fn undoable(&self) -> bool {
        !matches!(self, Change::Trash { .. })
    }
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub id: i64,
    /// Milliseconds since 1970.
    pub at: i64,
    pub label: String,
    pub kind: String,
    pub undoable: bool,
    pub undone: bool,
    /// What happened when it was undone, if not everything could be.
    pub note: Option<String>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

fn photos(n: usize) -> String {
    if n == 1 { "1 photo".into() } else { format!("{n} photos") }
}

/// Remember a change. Returns its history id.
pub fn record(conn: &Connection, label: &str, change: &Change) -> Result<i64> {
    conn.execute(
        "INSERT INTO history (at, label, change) VALUES (?1, ?2, ?3)",
        params![now(), label, serde_json::to_string(change)?],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute("DELETE FROM history WHERE id <= ?1", [id - KEEP])?;
    Ok(id)
}

// ------------------------------------------------------------------ recording each kind

pub fn record_move(conn: &Connection, rep: &mut OpReport, copy: bool, dest: &Path) -> Result<()> {
    let to = file_name(dest);
    if copy && !rep.copies.is_empty() {
        let n = rep.done;
        rep.history = Some(record(conn, &format!("Copied {} to {to}", photos(n)), &Change::Copy { files: rep.copies.clone() })?);
    } else if !copy && !rep.moved.is_empty() {
        let n = rep.moved.len();
        let label = if n == 1 {
            format!("Moved {} to {to}", file_name(Path::new(&rep.moved[0].from)))
        } else {
            format!("Moved {} to {to}", photos(n))
        };
        rep.history = Some(record(conn, &label, &Change::Move { items: rep.moved.clone() })?);
    }
    Ok(())
}

pub fn record_rename(conn: &Connection, rep: &mut RenameReport) -> Result<()> {
    let label = match rep.renamed.as_slice() {
        [] => return Ok(()),
        [(_, old, new)] => format!("Renamed {old} to {new}"),
        many => format!("Renamed {}", photos(many.len())),
    };
    rep.history = Some(record(conn, &label, &Change::Rename { items: rep.renamed.clone() })?);
    Ok(())
}

pub fn record_trash(conn: &Connection, rep: &mut OpReport, permanent: bool) -> Result<()> {
    if rep.done == 0 {
        return Ok(());
    }
    let label = if permanent { format!("Deleted {} permanently", photos(rep.done)) } else { format!("Moved {} to the Trash", photos(rep.done)) };
    rep.history = Some(record(conn, &label, &Change::Trash { count: rep.done, permanent })?);
    Ok(())
}

// ------------------------------------------------------------------ listing

pub fn list(conn: &Connection, limit: i64) -> Result<Vec<Entry>> {
    let mut st = conn.prepare("SELECT id, at, label, change, undone_at, note FROM history ORDER BY id DESC LIMIT ?1")?;
    let rows = st.query_map([limit], |r| {
        let change: String = r.get(3)?;
        let undone: Option<i64> = r.get(4)?;
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, change, undone.is_some(), r.get(5)?))
    })?;
    let mut out = vec![];
    for row in rows {
        let (id, at, label, change, undone, note): (i64, i64, String, String, bool, Option<String>) = row?;
        let c: Option<Change> = serde_json::from_str(&change).ok();
        let kind = serde_json::from_str::<serde_json::Value>(&change)
            .ok()
            .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(String::from))
            .unwrap_or_default();
        out.push(Entry { id, at, label, kind, undoable: c.as_ref().is_some_and(Change::undoable), undone, note });
    }
    Ok(out)
}

// ------------------------------------------------------------------ undo

#[derive(Debug, Default, Serialize)]
pub struct UndoReport {
    /// History entries undone.
    pub steps: usize,
    /// Files or folders put back.
    pub done: usize,
    /// What couldn't be put back: (name, why).
    pub failed: Vec<(String, String)>,
    /// What the last step undone was, for the message ("Undid: Renamed 12 photos").
    pub label: Option<String>,
}

/// The newest change that can still be undone.
pub fn latest(conn: &Connection) -> Result<Option<i64>> {
    let mut st = conn.prepare("SELECT id, change FROM history WHERE undone_at IS NULL ORDER BY id DESC")?;
    let rows: Vec<(i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .into_iter()
        .find(|(_, c)| serde_json::from_str::<Change>(c).is_ok_and(|c| c.undoable()))
        .map(|(id, _)| id))
}

/// Undo the change `id` and every change made after it, newest first. Without an id,
/// undo just the latest change.
pub fn undo<H: Host + ?Sized>(h: &H, id: Option<i64>) -> Result<UndoReport> {
    let target = match id {
        Some(id) => id,
        None => match latest(&h.conn().lock())? {
            Some(id) => id,
            None => bail!("There is nothing to undo."),
        },
    };
    let steps: Vec<(i64, String, String)> = {
        let conn = h.conn().lock();
        let exists: Option<i64> = conn.query_row("SELECT id FROM history WHERE id = ?1", [target], |r| r.get(0)).optional()?;
        if exists.is_none() {
            bail!("That change is no longer in the history.");
        }
        let mut st = conn.prepare("SELECT id, label, change FROM history WHERE id >= ?1 AND undone_at IS NULL ORDER BY id DESC")?;
        let rows = st.query_map([target], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut rep = UndoReport::default();
    for (id, label, change) in steps {
        let Ok(change) = serde_json::from_str::<Change>(&change) else { continue };
        if !change.undoable() {
            continue;
        }
        let before = rep.failed.len();
        let done = undo_one(h, &change, &mut rep)?;
        let failed = rep.failed.len() - before;
        let note = match (done, failed) {
            (_, 0) => None,
            (0, f) => Some(format!("Nothing could be put back ({f} problem{}).", if f == 1 { "" } else { "s" })),
            (d, f) => Some(format!("{d} put back, {f} could not be.")),
        };
        rep.done += done;
        rep.steps += 1;
        rep.label = Some(label);
        h.conn().lock().execute("UPDATE history SET undone_at = ?1, note = ?2 WHERE id = ?3", params![now(), note, id])?;
    }
    Ok(rep)
}

fn undo_one<H: Host + ?Sized>(h: &H, change: &Change, rep: &mut UndoReport) -> Result<usize> {
    match change {
        Change::Move { items } => undo_move(h, items, rep),
        Change::Copy { files } => Ok(undo_copy(h, files, rep)),
        Change::Rename { items } => {
            // Only photos that still have the name this rename gave them.
            let mut back = vec![];
            for (id, old, new) in items {
                match db::get_photo(&h.conn().lock(), *id)? {
                    Some(p) if &p.filename == new => back.push((*id, old.clone())),
                    Some(p) => rep.failed.push((p.filename, "it has been renamed again since".into())),
                    None => rep.failed.push((new.clone(), "it is no longer in the library".into())),
                }
            }
            let r = rename::rename_in(h.conn(), &back)?;
            rep.failed.extend(r.failed);
            Ok(r.done)
        }
        Change::FolderRename { from, to } => {
            let name = file_name(from);
            if !to.is_dir() {
                rep.failed.push((file_name(to), "the folder isn't there any more (is its drive connected?)".into()));
                return Ok(0);
            }
            if from.exists() && !fileops::same_place(from, to) {
                rep.failed.push((file_name(to), format!("there is a folder named \"{name}\" again")));
                return Ok(0);
            }
            match fileops::rename_folder(h, to, &name) {
                Ok(_) => Ok(1),
                Err(e) => {
                    rep.failed.push((file_name(to), format!("{e:#}")));
                    Ok(0)
                }
            }
        }
        Change::FolderMove { from, to } => {
            let Some(parent) = from.parent() else { return Ok(0) };
            if !to.is_dir() {
                rep.failed.push((file_name(to), "the folder isn't there any more (is its drive connected?)".into()));
                return Ok(0);
            }
            if from.exists() {
                rep.failed.push((file_name(to), format!("there is a folder named \"{}\" in {} again", file_name(from), parent.display())));
                return Ok(0);
            }
            match fileops::move_folder(h, to, parent, false) {
                Ok(r) => {
                    // Back under its old name too, if the move had to number it.
                    if let Some(now_at) = r.folder {
                        if now_at != *from {
                            let _ = fileops::rename_folder(h, &now_at, &file_name(from));
                        }
                    }
                    Ok(1)
                }
                Err(e) => {
                    rep.failed.push((file_name(to), format!("{e:#}")));
                    Ok(0)
                }
            }
        }
        Change::FolderCopy { to } => {
            if !to.is_dir() {
                return Ok(0);
            }
            match trash::delete(to) {
                Ok(()) => {
                    h.queue(Job::Removed(vec![to.clone()]));
                    crate::notify::changed([to.as_path()]);
                    Ok(1)
                }
                Err(e) => {
                    rep.failed.push((file_name(to), format!("the copy could not be moved to the Trash ({e}); delete it yourself if you don't want it")));
                    Ok(0)
                }
            }
        }
        Change::FolderCreate { path } => match std::fs::remove_dir(path) {
            Ok(()) => {
                crate::notify::changed([path.as_path()]);
                h.queue(Job::Removed(vec![path.clone()]));
                Ok(1)
            }
            Err(_) if !path.exists() => Ok(1),
            Err(_) => {
                rep.failed.push((file_name(path), "something has been put in it since, so it was kept".into()));
                Ok(0)
            }
        },
        Change::Trash { .. } => Ok(0),
    }
}

fn undo_move<H: Host + ?Sized>(h: &H, items: &[Moved], rep: &mut UndoReport) -> Result<usize> {
    // Photos still in the library go back through the normal move (which also deals with
    // drives that are away), grouped by their old folder.
    let mut back: Vec<(i64, PathBuf, Option<String>)> = vec![];
    let mut done = 0;
    for m in items {
        let from = PathBuf::from(&m.from);
        let (Some(dir), Some(name)) = (from.parent(), from.file_name()) else { continue };
        let name = name.to_string_lossy().to_string();
        if !m.in_library {
            match fileops::move_file_back(h, Path::new(&m.to), dir, &name) {
                Ok(_) => done += 1,
                Err(e) => rep.failed.push((file_name(Path::new(&m.to)), format!("{e:#}"))),
            }
            continue;
        }
        match db::get_photo(&h.conn().lock(), m.id)? {
            Some(p) if p.path == m.to => back.push((m.id, dir.to_path_buf(), Some(name))),
            Some(p) => rep.failed.push((p.filename, "it has been moved or renamed again since".into())),
            None => rep.failed.push((file_name(Path::new(&m.to)), "it is no longer in the library".into())),
        }
    }
    // Same folder together keeps the progress messages tidy.
    let mut by_dir: BTreeMap<PathBuf, Vec<(i64, PathBuf, Option<String>)>> = BTreeMap::new();
    for b in back {
        by_dir.entry(b.1.clone()).or_default().push(b);
    }
    for (dir, group) in by_dir {
        let r = match fileops::move_photos_to(h, &group, false) {
            Ok(r) => r,
            Err(e) => {
                // The whole folder is unavailable.
                let msg = format!("{e:#}");
                for (id, _, _) in &group {
                    let name = db::get_photo(&h.conn().lock(), *id)?.map(|p| p.filename).unwrap_or_default();
                    rep.failed.push((name, msg.clone()));
                }
                log::warn!("undo move to {}: {msg}", dir.display());
                continue;
            }
        };
        done += r.done + r.queued;
        rep.failed.extend(r.failed);
    }
    Ok(done)
}

/// Copies go to the Trash. Where a drive has no Trash (network drives often don't), a copy
/// is deleted only if it is still identical to its original, so nothing unique is lost.
fn undo_copy<H: Host + ?Sized>(h: &H, files: &[(PathBuf, PathBuf)], rep: &mut UndoReport) -> usize {
    let mut done = 0;
    let mut removed = vec![];
    for (copy, original) in files {
        if !copy.exists() {
            continue;
        }
        let ok = match trash::delete(copy) {
            Ok(()) => true,
            Err(_) => match fileops::same_content(copy, original) {
                Ok(true) => std::fs::remove_file(copy).is_ok(),
                _ => {
                    rep.failed.push((file_name(copy), "the drive has no Trash and the copy is no longer the same as the original, so it was kept".into()));
                    false
                }
            },
        };
        if ok {
            removed.push(copy.clone());
            // Sidecars aren't counted as photos.
            if crate::indexer::ext_of(copy) != "xmp" {
                done += 1;
            }
        }
    }
    if !removed.is_empty() {
        crate::notify::changed(&removed);
        h.queue(Job::Removed(removed));
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fileops::testing::*;

    fn entries(h: &TestHost) -> Vec<Entry> {
        list(&h.conn.lock(), 50).unwrap()
    }

    #[test]
    fn undo_a_move_puts_photos_and_sidecars_back() {
        let t = Tmp::new("hist");
        let h = host();
        let r = h.root(&t.0);
        let a = h.file(r, &t.join("in/IMG_1.CR2"), None);
        h.file(r, &t.join("in/IMG_1.xmp"), None);
        let b = h.file(r, &t.join("in2/IMG_1.CR2"), Some("other"));
        std::fs::create_dir_all(t.join("out")).unwrap();
        let mut rep = fileops::move_photos(&h, &[a, b], &t.join("out"), false).unwrap();
        record_move(&h.conn.lock(), &mut rep, false, &t.join("out")).unwrap();
        assert_eq!(names(&t.join("out")), ["IMG_1 (2).CR2", "IMG_1.CR2", "IMG_1.xmp"]);
        let e = entries(&h);
        assert_eq!(e[0].label, "Moved 2 photos to out");
        let u = undo(&h, None).unwrap();
        assert_eq!((u.steps, u.done, u.failed.len()), (1, 2, 0));
        assert_eq!(names(&t.join("in")), ["IMG_1.CR2", "IMG_1.xmp"]);
        assert_eq!(read(&t.join("in2/IMG_1.CR2")), "other", "back under its own name, not the numbered one");
        assert!(names(&t.join("out")).is_empty());
        assert_eq!(h.path_of(b).unwrap(), t.join("in2/IMG_1.CR2").to_string_lossy());
        assert_eq!(h.rating(a), 4);
        assert!(entries(&h)[0].undone);
        assert!(undo(&h, None).is_err(), "nothing left to undo");
    }

    #[test]
    fn undo_several_steps_in_order() {
        let t = Tmp::new("hist");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("a/IMG_1.jpg"), None);
        std::fs::create_dir_all(t.join("b")).unwrap();
        std::fs::create_dir_all(t.join("c")).unwrap();
        // Rename, then move, then move again.
        let mut rr = rename::rename_in(&h.conn, &[(id, "Beach.jpg".into())]).unwrap();
        record_rename(&h.conn.lock(), &mut rr).unwrap();
        let mut m1 = fileops::move_photos(&h, &[id], &t.join("b"), false).unwrap();
        record_move(&h.conn.lock(), &mut m1, false, &t.join("b")).unwrap();
        let mut m2 = fileops::move_photos(&h, &[id], &t.join("c"), false).unwrap();
        record_move(&h.conn.lock(), &mut m2, false, &t.join("c")).unwrap();
        let labels: Vec<String> = entries(&h).into_iter().map(|e| e.label).collect();
        assert_eq!(labels, ["Moved Beach.jpg to c", "Moved Beach.jpg to b", "Renamed IMG_1.jpg to Beach.jpg"]);
        // One step back: in b.
        undo(&h, None).unwrap();
        assert!(t.join("b/Beach.jpg").exists());
        // Back to the rename: everything after it is undone first.
        let u = undo(&h, rr.history).unwrap();
        assert_eq!(u.steps, 2);
        assert_eq!(names(&t.join("a")), ["IMG_1.jpg"]);
        assert_eq!(h.path_of(id).unwrap(), t.join("a/IMG_1.jpg").to_string_lossy());
        assert!(entries(&h).iter().all(|e| e.undone));
    }

    #[test]
    fn undo_leaves_photos_alone_that_changed_since() {
        let t = Tmp::new("hist");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("a/IMG_1.jpg"), None);
        let mut rr = rename::rename_in(&h.conn, &[(id, "One.jpg".into())]).unwrap();
        record_rename(&h.conn.lock(), &mut rr).unwrap();
        // Renamed again without the history knowing (say an older version of Lumio).
        rename::rename_in(&h.conn, &[(id, "Two.jpg".into())]).unwrap();
        let u = undo(&h, None).unwrap();
        assert_eq!((u.done, u.failed.len()), (0, 1));
        assert_eq!(names(&t.join("a")), ["Two.jpg"]);
        assert!(entries(&h)[0].note.as_deref().unwrap().contains("Nothing could be put back"));
    }

    #[test]
    fn undo_never_overwrites_a_file_that_took_the_old_name() {
        let t = Tmp::new("hist");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("a/IMG_1.jpg"), Some("mine"));
        std::fs::create_dir_all(t.join("b")).unwrap();
        let mut m = fileops::move_photos(&h, &[id], &t.join("b"), false).unwrap();
        record_move(&h.conn.lock(), &mut m, false, &t.join("b")).unwrap();
        std::fs::write(t.join("a/IMG_1.jpg"), "new arrival").unwrap();
        undo(&h, None).unwrap();
        assert_eq!(read(&t.join("a/IMG_1.jpg")), "new arrival");
        assert_eq!(read(&t.join("a/IMG_1 (2).jpg")), "mine");
    }

    #[test]
    fn undo_a_move_to_a_drive_that_is_away_cancels_it() {
        let t = Tmp::new("hist");
        let h = host();
        let ra = h.root(&t.join("a"));
        h.root(&t.join("b")); // away
        let id = h.file(ra, &t.join("a/IMG_1.jpg"), None);
        let mut m = fileops::move_photos(&h, &[id], &t.join("b/Trip"), false).unwrap();
        assert_eq!(m.queued, 1);
        record_move(&h.conn.lock(), &mut m, false, &t.join("b/Trip")).unwrap();
        let u = undo(&h, None).unwrap();
        assert_eq!(u.done, 1, "{:?}", u.failed);
        assert_eq!(crate::pending::count(&h.conn.lock()), 0, "the waiting move is gone");
        assert_eq!(h.path_of(id).unwrap(), t.join("a/IMG_1.jpg").to_string_lossy());
        assert!(t.join("a/IMG_1.jpg").exists());
    }

    #[test]
    fn undo_a_move_made_while_the_photos_drive_was_away() {
        // Photo on drive a (away) moved to b (here). Undo while a is still away: the
        // move back also waits, and cancels the first one.
        let t = Tmp::new("hist");
        let h = host();
        let ra = h.root(&t.join("a"));
        let rb = h.root(&t.join("b"));
        std::fs::create_dir_all(t.join("b")).unwrap();
        let id = h.photo(ra, &t.join("a/IMG_1.jpg"));
        let mut m = fileops::move_photos(&h, &[id], &t.join("b"), false).unwrap();
        assert_eq!(m.queued, 1);
        record_move(&h.conn.lock(), &mut m, false, &t.join("b")).unwrap();
        let _ = rb;
        let u = undo(&h, None).unwrap();
        assert_eq!(u.done, 1, "{:?}", u.failed);
        assert_eq!(crate::pending::count(&h.conn.lock()), 0);
        assert_eq!(h.path_of(id).unwrap(), t.join("a/IMG_1.jpg").to_string_lossy());
    }

    #[test]
    fn undo_a_batch_rename_with_swaps() {
        let t = Tmp::new("hist");
        let h = host();
        let r = h.root(&t.0);
        let a = h.file(r, &t.join("a.jpg"), None);
        let b = h.file(r, &t.join("b.jpg"), None);
        let c = h.file(r, &t.join("c.jpg"), None);
        h.file(r, &t.join("c.xmp"), None);
        let mut rr = rename::rename_in(&h.conn, &[(a, "b.jpg".into()), (b, "a.jpg".into()), (c, "Sunset 001.jpg".into())]).unwrap();
        record_rename(&h.conn.lock(), &mut rr).unwrap();
        assert_eq!(entries(&h)[0].label, "Renamed 3 photos");
        undo(&h, None).unwrap();
        assert_eq!(names(&t.0), ["a.jpg", "b.jpg", "c.jpg", "c.xmp"]);
        assert_eq!(read(&t.join("a.jpg")), "a.jpg");
        assert_eq!(read(&t.join("c.xmp")), "c.xmp");
    }

    #[test]
    fn undo_a_copy_removes_only_the_copies() {
        let t = Tmp::new("hist");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("in/IMG_1.jpg"), None);
        h.file(r, &t.join("in/IMG_1.xmp"), None);
        std::fs::create_dir_all(t.join("out")).unwrap();
        let mut c = fileops::move_photos(&h, &[id], &t.join("out"), true).unwrap();
        record_move(&h.conn.lock(), &mut c, true, &t.join("out")).unwrap();
        assert_eq!(entries(&h)[0].label, "Copied 1 photo to out");
        let u = undo(&h, None).unwrap();
        // The test machine may have no Trash; then identical copies are deleted instead.
        assert_eq!(u.done, 1, "{:?}", u.failed);
        assert!(names(&t.join("out")).is_empty());
        assert_eq!(names(&t.join("in")), ["IMG_1.jpg", "IMG_1.xmp"]);
    }

    #[test]
    fn undo_folder_rename_move_and_create() {
        let t = Tmp::new("hist");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("Trip/IMG_1.jpg"), None);
        std::fs::create_dir_all(t.join("Travel")).unwrap();
        let conn = || h.conn.lock();
        let made = fileops::create_folder(&t.0, "Empty").unwrap();
        record(&conn(), "Created folder Empty", &Change::FolderCreate { path: made }).unwrap();
        let renamed = fileops::rename_folder(&h, &t.join("Trip"), "Iceland").unwrap();
        record(&conn(), "Renamed folder", &Change::FolderRename { from: t.join("Trip"), to: renamed.clone() }).unwrap();
        let moved = fileops::move_folder(&h, &renamed, &t.join("Travel"), false).unwrap().folder.unwrap();
        record(&conn(), "Moved folder", &Change::FolderMove { from: renamed.clone(), to: moved.clone() }).unwrap();
        assert_eq!(h.path_of(id).unwrap(), t.join("Travel/Iceland/IMG_1.jpg").to_string_lossy());
        let first = entries(&h).last().unwrap().id;
        let u = undo(&h, Some(first)).unwrap();
        assert_eq!((u.steps, u.done, u.failed.len()), (3, 3, 0), "{:?}", u.failed);
        assert_eq!(names(&t.0), ["Travel", "Trip"]);
        assert_eq!(h.path_of(id).unwrap(), t.join("Trip/IMG_1.jpg").to_string_lossy());
    }

    #[test]
    fn a_new_folder_with_files_in_it_is_kept() {
        let t = Tmp::new("hist");
        let h = host();
        let made = fileops::create_folder(&t.0, "New").unwrap();
        record(&h.conn.lock(), "Created folder New", &Change::FolderCreate { path: made.clone() }).unwrap();
        std::fs::write(made.join("photo.jpg"), "x").unwrap();
        let u = undo(&h, None).unwrap();
        assert_eq!((u.done, u.failed.len()), (0, 1));
        assert!(made.join("photo.jpg").exists());
    }

    #[test]
    fn trash_is_listed_but_skipped() {
        let h = host();
        let c = h.conn.lock();
        let a = record(&c, "Created folder X", &Change::FolderCreate { path: "/nonexistent/x".into() }).unwrap();
        record(&c, "Moved 2 photos to the Trash", &Change::Trash { count: 2, permanent: false }).unwrap();
        assert_eq!(latest(&c).unwrap(), Some(a));
        let e = list(&c, 10).unwrap();
        assert!(!e[0].undoable && e[1].undoable);
        assert_eq!(e[0].kind, "trash");
    }

    #[test]
    fn history_is_kept_to_a_limit() {
        let h = host();
        let c = h.conn.lock();
        for i in 0..(KEEP + 20) {
            record(&c, &format!("{i}"), &Change::Trash { count: 1, permanent: false }).unwrap();
        }
        let n: i64 = c.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap();
        assert_eq!(n, KEEP);
    }
}
