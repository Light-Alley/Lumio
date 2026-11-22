//! Moves made while a drive is away.
//!
//! When a photo is moved and its drive (or the destination's drive) isn't connected,
//! the catalogue moves it straight away, so it shows in its new folder and can be
//! previewed there, and the move itself waits in `pending_moves` with the place the
//! file really is (`src`). When both drives are back, the worker does the real move.
//!
//! Nothing is ever overwritten: if a file with the same name has turned up in the
//! destination, the photo gets a free name like "IMG_0001 (2).jpg". Moves between
//! drives are copy, check, then delete (see `fileops::move_file`). If the original has
//! gone missing in the meantime the move is kept, with a note, until the user cancels it.

use crate::db;
use crate::fileops;
use crate::indexer::{self, find_sidecar};
use crate::state::AppState;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tauri::Emitter;

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pending_moves (
            photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
            src TEXT NOT NULL,
            created INTEGER NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            error TEXT
        );",
    )?;
    Ok(())
}

/// Where the file for a photo with a waiting move really is.
pub fn source_of(conn: &Connection, photo_id: i64) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT src FROM pending_moves WHERE photo_id = ?1", [photo_id], |r| r.get(0)).optional()?)
}

/// Files that are waiting to be moved, by where they really are. The indexer must not
/// add these again as new photos when it sees them.
pub fn sources(conn: &Connection) -> Result<HashSet<String>> {
    let mut st = conn.prepare("SELECT src FROM pending_moves")?;
    let rows = st.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Photos with a waiting move. Their catalogue path doesn't exist yet, so scans must not
/// forget them.
pub fn photo_ids(conn: &Connection) -> Result<HashSet<i64>> {
    let mut st = conn.prepare("SELECT photo_id FROM pending_moves")?;
    let rows = st.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM pending_moves", [], |r| r.get(0)).unwrap_or(0)
}

fn split(path: &Path) -> (String, String) {
    (
        path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
    )
}

/// A name in `dir` that neither the catalogue nor the disk (if we can see it) uses, for
/// the photo and for its sidecar when it has one.
fn free_target(conn: &Connection, dir: &Path, name: &str, photo_id: i64, sidecar: Option<&Path>) -> Result<PathBuf> {
    let mut err = None;
    let found = fileops::free_name(dir, name, sidecar, |p| {
        let other: rusqlite::Result<Option<i64>> =
            conn.query_row("SELECT id FROM photos WHERE path = ?1", [p.to_string_lossy()], |r| r.get(0)).optional();
        match other {
            Ok(other) => other.is_some_and(|id| id != photo_id) || p.exists(),
            Err(e) => {
                err = Some(e);
                false
            }
        }
    });
    match err {
        Some(e) => Err(e.into()),
        None => Ok(found),
    }
}

/// Move a photo in the catalogue now and remember to move the file later.
/// Returns where it will end up.
/// `name` is the file name it should have there (normally the one it has now).
pub fn queue_move(conn: &Connection, photo: &db::Photo, dest: &Path, name: &str, dest_root: i64) -> Result<PathBuf> {
    let src = source_of(conn, photo.id)?.unwrap_or_else(|| photo.path.clone());
    // Moving it back to where the file really is cancels the move.
    if Path::new(&src).parent() == Some(dest)
        && Path::new(&src).file_name().is_some_and(|n| n.to_string_lossy() == name)
    {
        cancel_one(conn, photo.id)?;
        return Ok(PathBuf::from(src));
    }
    let target = free_target(conn, dest, name, photo.id, None)?;
    let (folder, filename) = split(&target);
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE photos SET path = ?1, folder = ?2, filename = ?3, root_id = ?4 WHERE id = ?5",
        params![target.to_string_lossy(), folder, filename, dest_root, photo.id],
    )?;
    tx.execute(
        "INSERT INTO pending_moves (photo_id, src, created) VALUES (?1, ?2, ?3)
         ON CONFLICT(photo_id) DO UPDATE SET attempts = 0, error = NULL",
        params![photo.id, src, crate::db::now_ms()],
    )?;
    tx.commit()?;
    Ok(target)
}

/// Undo a waiting move: the photo goes back to where its file really is.
fn cancel_one(conn: &Connection, photo_id: i64) -> Result<()> {
    let Some(src) = source_of(conn, photo_id)? else { return Ok(()) };
    let sp = PathBuf::from(&src);
    let (folder, filename) = split(&sp);
    let root = db::root_for_path(conn, &sp)?.map(|(id, _)| id);
    let tx = conn.unchecked_transaction()?;
    // If the original was meanwhile catalogued again on its own, keep that entry's
    // place and drop this one's (the file is the same).
    let dup: Option<i64> = tx
        .query_row("SELECT id FROM photos WHERE path = ?1 AND id <> ?2", params![src, photo_id], |r| r.get(0))
        .optional()?;
    if dup.is_some() {
        tx.execute("DELETE FROM photos WHERE id = ?1", [photo_id])?;
    } else {
        tx.execute(
            "UPDATE photos SET path = ?1, folder = ?2, filename = ?3, root_id = coalesce(?4, root_id) WHERE id = ?5",
            params![src, folder, filename, root, photo_id],
        )?;
        tx.execute("DELETE FROM pending_moves WHERE photo_id = ?1", [photo_id])?;
    }
    tx.commit()?;
    Ok(())
}

pub fn cancel(state: &AppState, photo_ids: &[i64]) -> Result<usize> {
    let conn = state.conn.lock();
    let mut n = 0;
    for id in photo_ids {
        if source_of(&conn, *id)?.is_some() {
            cancel_one(&conn, *id)?;
            n += 1;
        }
    }
    Ok(n)
}

#[derive(Debug, Serialize)]
pub struct PendingMove {
    pub photo_id: i64,
    pub filename: String,
    pub from: String,
    pub to: String,
    /// Why the last try didn't work, if it was tried.
    pub error: Option<String>,
}

pub fn list(conn: &Connection) -> Result<Vec<PendingMove>> {
    let mut st = conn.prepare(
        "SELECT m.photo_id, p.filename, m.src, p.folder, m.error FROM pending_moves m JOIN photos p ON p.id = m.photo_id ORDER BY m.created",
    )?;
    let rows = st.query_map([], |r| {
        let src: String = r.get(2)?;
        Ok(PendingMove {
            photo_id: r.get(0)?,
            filename: r.get(1)?,
            from: Path::new(&src).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
            to: r.get(3)?,
            error: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Default, Serialize, Clone)]
pub struct ApplyReport {
    /// Moves finished this time.
    pub done: usize,
    /// Moves that couldn't be done, newly or with a new reason: (file name, why).
    pub failed: Vec<(String, String)>,
    /// Moves still waiting (drive not connected, or failed).
    pub waiting: usize,
}

/// Is the drive this path lives on connected? Answers are cached per library folder.
struct Reach<'a> {
    conn: &'a Connection,
    seen: HashMap<PathBuf, bool>,
}

impl Reach<'_> {
    fn ok(&mut self, path: &Path) -> Result<bool> {
        // A file outside the library: judge by its folder.
        let base = match db::root_for_path(self.conn, path)? {
            Some((_, root)) => root,
            None => path.parent().map(Path::to_path_buf).unwrap_or_default(),
        };
        if let Some(v) = self.seen.get(&base) {
            return Ok(*v);
        }
        let v = indexer::reachable(&base);
        self.seen.insert(base, v);
        Ok(v)
    }
}

/// Do every waiting move whose drives are both connected, and tell the window.
pub fn apply(state: &AppState, conn: &Connection) -> Result<ApplyReport> {
    let rep = apply_in(conn)?;
    if rep.done > 0 || !rep.failed.is_empty() {
        let _ = state.app.emit("pending-moves", rep.clone());
        let _ = state.app.emit("library-changed", ());
    }
    Ok(rep)
}

/// Do every waiting move whose drives are both connected.
pub fn apply_in(conn: &Connection) -> Result<ApplyReport> {
    let rows: Vec<(i64, String, String, String, i64, Option<String>)> = {
        let mut st = conn.prepare(
            "SELECT m.photo_id, m.src, p.path, p.filename, p.size, m.error FROM pending_moves m JOIN photos p ON p.id = m.photo_id ORDER BY m.created",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut rep = ApplyReport::default();
    if rows.is_empty() {
        return Ok(rep);
    }
    let mut reach = Reach { conn, seen: HashMap::new() };
    let mut touched: Vec<PathBuf> = vec![];
    for (id, src, dest, filename, size, old_err) in rows {
        let src = PathBuf::from(src);
        let dest = PathBuf::from(dest);
        if !reach.ok(&src)? || !reach.ok(&dest)? {
            rep.waiting += 1;
            continue;
        }
        let res = move_one(conn, id, &src, &dest, &filename, size);
        match res {
            Ok(Some(target)) => {
                rep.done += 1;
                log::info!("waiting move done: {} -> {}", src.display(), target.display());
                touched.extend(src.parent().map(Path::to_path_buf));
                touched.extend(target.parent().map(Path::to_path_buf));
            }
            Ok(None) => rep.done += 1,
            Err(e) => {
                let msg = format!("{e:#}");
                log::warn!("waiting move of {} failed: {msg}", src.display());
                conn.execute(
                    "UPDATE pending_moves SET attempts = attempts + 1, error = ?2 WHERE photo_id = ?1",
                    params![id, msg],
                )?;
                rep.waiting += 1;
                if old_err.as_deref() != Some(msg.as_str()) {
                    rep.failed.push((filename, msg));
                }
            }
        }
    }
    if !touched.is_empty() {
        crate::notify::changed(&touched);
    }
    Ok(rep)
}

/// Returns where the file ended up, or None if it was already there.
fn move_one(conn: &Connection, id: i64, src: &Path, dest: &Path, filename: &str, size: i64) -> Result<Option<PathBuf>> {
    let dir = dest.parent().context("no destination folder")?;
    if !src.exists() {
        // Moved by hand in the meantime?
        if std::fs::metadata(dest).map(|m| m.len() as i64 == size).unwrap_or(false) {
            conn.execute("DELETE FROM pending_moves WHERE photo_id = ?1", [id])?;
            return Ok(None);
        }
        anyhow::bail!(
            "The original wasn't found in {}. It may have been moved or deleted outside Lumio.",
            src.parent().map(|p| p.display().to_string()).unwrap_or_default()
        );
    }
    // The folder may have been removed while the drive was away; make it again.
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let side = find_sidecar(src);
    // Nothing is overwritten: not a file that turned up with the same name while the
    // drive was away, and not another photo's sidecar.
    let side_taken = side.as_ref().is_some_and(|s| {
        let t = fileops::sidecar_target(dest, s);
        t.exists() && !fileops::same_content(s, &t).unwrap_or(false)
    });
    let target = if dest.exists() || side_taken {
        let name = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| filename.to_string());
        free_target(conn, dir, &name, id, side.as_deref())?
    } else {
        dest.to_path_buf()
    };
    fileops::move_file(src, &target)?;
    if let Some(s) = &side {
        let leaving: HashSet<PathBuf> = [src.to_path_buf()].into();
        let t = fileops::sidecar_target(&target, s);
        if let Err(e) = fileops::carry_sidecar(s, &t, fileops::sidecar_shared(src, s, &leaving)) {
            log::warn!("sidecar {} could not follow its photo: {e:#}", s.display());
        }
    }
    let (folder, name) = split(&target);
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE photos SET path = ?1, folder = ?2, filename = ?3 WHERE id = ?4",
        params![target.to_string_lossy(), folder, name, id],
    )?;
    tx.execute("DELETE FROM pending_moves WHERE photo_id = ?1", [id])?;
    tx.commit()?;
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir() -> Tmp {
        Tmp(tempfile::Builder::new().prefix("lumio-pending-").tempdir().unwrap().keep())
    }

    fn setup(dir: &Path) -> Connection {
        let conn = db::open(&dir.join("c.sqlite")).unwrap();
        db::migrate(&conn).unwrap();
        conn
    }

    fn add_root(conn: &Connection, path: &Path) -> i64 {
        conn.execute("INSERT INTO roots (path, name, added_at) VALUES (?1, 'r', 0)", [path.to_string_lossy()]).unwrap();
        conn.last_insert_rowid()
    }

    fn add_photo(conn: &Connection, root: i64, path: &Path, size: i64) -> i64 {
        let (folder, name) = split(path);
        conn.execute(
            "INSERT INTO photos (root_id, path, folder, filename, ext, size) VALUES (?1, ?2, ?3, ?4, 'jpg', ?5)",
            params![root, path.to_string_lossy(), folder, name, size],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn queue_then_move_without_overwriting() {
        let t = tempdir();
        let conn = setup(t.path());
        let a = t.path().join("a");
        let b = t.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        let ra = add_root(&conn, &a);
        let rb = add_root(&conn, &b);
        let src = a.join("IMG_1.jpg");
        std::fs::write(&src, b"photo").unwrap();
        std::fs::write(a.join("IMG_1.xmp"), b"side").unwrap();
        let id = add_photo(&conn, ra, &src, 5);
        let photo = db::get_photo(&conn, id).unwrap().unwrap();
        // Drive b is "away": the move waits, but the catalogue shows the new place.
        let planned = queue_move(&conn, &photo, &b, &photo.filename, rb).unwrap();
        assert_eq!(planned, b.join("IMG_1.jpg"));
        assert_eq!(db::get_photo(&conn, id).unwrap().unwrap().folder, b.to_string_lossy());
        assert!(sources(&conn).unwrap().contains(&src.to_string_lossy().to_string()));
        // While away, someone put a different file with the same name there.
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("IMG_1.jpg"), b"other one").unwrap();
        let mut reach = Reach { conn: &conn, seen: HashMap::new() };
        assert!(reach.ok(&b.join("x")).unwrap());
        let got = move_one(&conn, id, &src, &planned, "IMG_1.jpg", 5).unwrap().unwrap();
        assert_eq!(got, b.join("IMG_1 (2).jpg"));
        assert_eq!(std::fs::read(b.join("IMG_1.jpg")).unwrap(), b"other one");
        assert_eq!(std::fs::read(&got).unwrap(), b"photo");
        assert!(b.join("IMG_1 (2).xmp").exists());
        assert!(!src.exists());
        assert_eq!(count(&conn), 0);
        assert_eq!(db::get_photo(&conn, id).unwrap().unwrap().filename, "IMG_1 (2).jpg");
    }

    #[test]
    fn missing_original_is_kept_and_can_be_cancelled() {
        let t = tempdir();
        let conn = setup(t.path());
        let a = t.path().join("a");
        let b = t.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let ra = add_root(&conn, &a);
        let rb = add_root(&conn, &b);
        let src = a.join("IMG_2.jpg");
        let id = add_photo(&conn, ra, &src, 5);
        let photo = db::get_photo(&conn, id).unwrap().unwrap();
        let planned = queue_move(&conn, &photo, &b, &photo.filename, rb).unwrap();
        assert!(move_one(&conn, id, &src, &planned, "IMG_2.jpg", 5).is_err());
        assert_eq!(count(&conn), 1);
        cancel_one(&conn, id).unwrap();
        let p = db::get_photo(&conn, id).unwrap().unwrap();
        assert_eq!(p.path, src.to_string_lossy());
        assert_eq!(p.root_id, ra);
        assert_eq!(count(&conn), 0);
    }

    #[test]
    fn moving_back_cancels() {
        let t = tempdir();
        let conn = setup(t.path());
        let a = t.path().join("a");
        let b = t.path().join("b");
        let ra = add_root(&conn, &a);
        let rb = add_root(&conn, &b);
        let src = a.join("IMG_3.jpg");
        let id = add_photo(&conn, ra, &src, 5);
        let photo = db::get_photo(&conn, id).unwrap().unwrap();
        queue_move(&conn, &photo, &b, &photo.filename, rb).unwrap();
        let photo = db::get_photo(&conn, id).unwrap().unwrap();
        queue_move(&conn, &photo, &a, &photo.filename, ra).unwrap();
        assert_eq!(count(&conn), 0);
        assert_eq!(db::get_photo(&conn, id).unwrap().unwrap().path, src.to_string_lossy());
    }

    #[test]
    fn already_moved_by_hand() {
        let t = tempdir();
        let conn = setup(t.path());
        let a = t.path().join("a");
        let b = t.path().join("b");
        std::fs::create_dir_all(&b).unwrap();
        let ra = add_root(&conn, &a);
        let rb = add_root(&conn, &b);
        let src = a.join("IMG_4.jpg");
        let id = add_photo(&conn, ra, &src, 5);
        let photo = db::get_photo(&conn, id).unwrap().unwrap();
        let planned = queue_move(&conn, &photo, &b, &photo.filename, rb).unwrap();
        std::fs::write(&planned, b"photo").unwrap();
        assert_eq!(move_one(&conn, id, &src, &planned, "IMG_4.jpg", 5).unwrap(), None);
        assert_eq!(count(&conn), 0);
    }

    #[test]
    fn waits_while_a_drive_is_away_and_never_overwrites_a_sidecar() {
        let t = tempdir();
        let conn = setup(t.path());
        let a = t.path().join("a");
        let b = t.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        let ra = add_root(&conn, &a);
        let rb = add_root(&conn, &b);
        let src = a.join("IMG_5.jpg");
        std::fs::write(&src, b"photo").unwrap();
        std::fs::write(a.join("IMG_5.xmp"), b"mine").unwrap();
        let id = add_photo(&conn, ra, &src, 5);
        let photo = db::get_photo(&conn, id).unwrap().unwrap();
        queue_move(&conn, &photo, &b, &photo.filename, rb).unwrap();
        // Drive b is away: nothing happens, the move keeps waiting.
        let rep = apply_in(&conn).unwrap();
        assert_eq!((rep.done, rep.waiting), (0, 1));
        assert!(src.exists());
        // It comes back with an IMG_5.xmp that belongs to someone else's IMG_5.CR2.
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("IMG_5.CR2"), b"raw").unwrap();
        std::fs::write(b.join("IMG_5.xmp"), b"theirs").unwrap();
        let rep = apply_in(&conn).unwrap();
        assert_eq!(rep.done, 1, "{:?}", rep.failed);
        assert_eq!(std::fs::read(b.join("IMG_5.xmp")).unwrap(), b"theirs");
        assert_eq!(std::fs::read(b.join("IMG_5 (2).xmp")).unwrap(), b"mine");
        assert_eq!(std::fs::read(b.join("IMG_5 (2).jpg")).unwrap(), b"photo");
        assert_eq!(db::get_photo(&conn, id).unwrap().unwrap().filename, "IMG_5 (2).jpg");
    }

    #[test]
    fn two_waiting_moves_with_the_same_name_get_different_names() {
        let t = tempdir();
        let conn = setup(t.path());
        let a = t.path().join("a");
        let b = t.path().join("b");
        std::fs::create_dir_all(a.join("x")).unwrap();
        std::fs::create_dir_all(a.join("y")).unwrap();
        let ra = add_root(&conn, &a);
        let rb = add_root(&conn, &b);
        let mut ids = vec![];
        for (dir, body) in [("x", b"one" as &[u8]), ("y", b"two")] {
            let src = a.join(dir).join("IMG_1.jpg");
            std::fs::write(&src, body).unwrap();
            let id = add_photo(&conn, ra, &src, 3);
            let photo = db::get_photo(&conn, id).unwrap().unwrap();
            queue_move(&conn, &photo, &b, &photo.filename, rb).unwrap();
            ids.push(id);
        }
        assert_eq!(db::get_photo(&conn, ids[1]).unwrap().unwrap().filename, "IMG_1 (2).jpg");
        std::fs::create_dir_all(&b).unwrap();
        assert_eq!(apply_in(&conn).unwrap().done, 2);
        assert_eq!(std::fs::read(b.join("IMG_1.jpg")).unwrap(), b"one");
        assert_eq!(std::fs::read(b.join("IMG_1 (2).jpg")).unwrap(), b"two");
    }

    #[test]
    fn a_raw_sidecar_stays_behind_for_a_waiting_jpeg_move() {
        let t = tempdir();
        let conn = setup(t.path());
        let a = t.path().join("a");
        let b = t.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        let ra = add_root(&conn, &a);
        let rb = add_root(&conn, &b);
        std::fs::write(a.join("IMG_6.CR2"), b"raw").unwrap();
        std::fs::write(a.join("IMG_6.xmp"), b"edits").unwrap();
        let src = a.join("IMG_6.jpg");
        std::fs::write(&src, b"jpeg").unwrap();
        let id = add_photo(&conn, ra, &src, 4);
        let photo = db::get_photo(&conn, id).unwrap().unwrap();
        queue_move(&conn, &photo, &b, &photo.filename, rb).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_eq!(apply_in(&conn).unwrap().done, 1);
        assert!(a.join("IMG_6.xmp").exists(), "the RAW keeps its sidecar");
        // "IMG_6.xmp" is the RAW's, so the JPEG goes without it.
        assert!(!b.join("IMG_6.xmp").exists());
    }
}
