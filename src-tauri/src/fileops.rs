//! Moving, copying, renaming and deleting photos and folders, keeping the catalogue in sync.
//!
//! The rules that keep originals safe:
//! - Nothing is ever overwritten. If a name is taken, the photo gets a free one like
//!   "IMG_0001 (2).jpg", and that includes its XMP sidecar's name.
//! - Moves between drives are copy -> compare every byte -> delete, so nothing is lost if
//!   a drive disconnects part-way through. A half-made copy is removed again.
//! - A sidecar shared by a RAW and its JPEG ("IMG_1.xmp") is copied, not taken away, when
//!   only one of the pair moves, and stays put when only one of them goes to the Trash.
//!
//! What each operation did is returned in its report so it can go in the undo history
//! (see `history.rs`).

use crate::db;
use crate::indexer::{self, find_sidecar, Job};
use crate::metadata;
use crate::state::AppState;
use anyhow::{anyhow, bail, Context, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tauri::Emitter;

/// What file operations need from the running app. `AppState` provides it; tests use a
/// stand-in, so the operations can be checked on real files without a window.
pub trait Host {
    fn conn(&self) -> &Mutex<Connection>;
    /// Progress for the status bar.
    fn progress(&self, _done: usize, _total: usize, _message: &str) {}
    /// Work for the background indexer.
    fn queue(&self, _job: Job) {}
    /// Remove photos from the catalogue, with their thumbnails.
    fn forget(&self, conn: &Connection, ids: &[i64]) -> Result<()>;
}

impl Host for AppState {
    fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }
    fn progress(&self, done: usize, total: usize, message: &str) {
        let _ = self.app.emit("op-progress", serde_json::json!({ "done": done, "total": total, "message": message }));
    }
    fn queue(&self, job: Job) {
        AppState::queue(self, job)
    }
    fn forget(&self, conn: &Connection, ids: &[i64]) -> Result<()> {
        indexer::delete_ids(self, conn, ids)
    }
}

/// One photo that was moved (or that will move when its drive is back).
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
pub struct Moved {
    pub id: i64,
    /// Where the catalogue had it before.
    pub from: String,
    /// Where it is now.
    pub to: String,
    /// False when it went to a folder outside the library and left the catalogue.
    pub in_library: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct OpReport {
    pub done: usize,
    pub failed: Vec<(String, String)>,
    /// True when the destination is not inside any library folder.
    pub outside_library: bool,
    /// For deletions: the photo id of each entry in `failed`, in the same order, so a
    /// retry can never pick a different photo that happens to share a file name.
    #[serde(default)]
    pub failed_ids: Vec<i64>,
    /// Moves that will happen when a drive that isn't connected comes back.
    #[serde(default)]
    pub queued: usize,
    /// The undo history entry for this operation, if anything changed.
    pub history: Option<i64>,
    /// Photos moved (for the undo history).
    #[serde(skip)]
    pub moved: Vec<Moved>,
    /// Files made by a copy: (copy, original). For the undo history.
    #[serde(skip)]
    pub copies: Vec<(PathBuf, PathBuf)>,
    /// Where a moved or copied folder ended up.
    #[serde(skip)]
    pub folder: Option<PathBuf>,
}

// ------------------------------------------------------------------ safe file primitives

/// Do two paths name the same place? Names that differ only in case are the same place
/// on drives that ignore case (macOS, Windows, exFAT cards) and different files on the
/// case-sensitive drives Linux usually has. When both exist the drive itself answers;
/// otherwise the usual behaviour of the system decides.
pub(crate) fn same_place(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if a.to_string_lossy().to_lowercase() != b.to_string_lossy().to_lowercase() {
        return false;
    }
    same_file(a, b).unwrap_or(cfg!(any(target_os = "macos", target_os = "windows")))
}

/// `Some(true)` when both paths exist and are the same file or folder.
fn same_file(a: &Path, b: &Path) -> Option<bool> {
    let (ma, mb) = (std::fs::metadata(a).ok()?, std::fs::metadata(b).ok()?);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(ma.dev() == mb.dev() && ma.ino() == mb.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (ma, mb);
        Some(std::fs::canonicalize(a).ok()? == std::fs::canonicalize(b).ok()?)
    }
}

/// Is `path` really inside `root` once links are followed? A link inside a library folder
/// that points elsewhere must not let folder commands reach outside the library. For a
/// path that doesn't exist yet the nearest existing parent is checked. When the root
/// itself isn't reachable (drive unplugged) there is nothing a link could lead to.
pub fn inside_root(root: &Path, path: &Path) -> Result<()> {
    let Ok(croot) = std::fs::canonicalize(root) else { return Ok(()) };
    let mut probe = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = vec![];
    let real = loop {
        match std::fs::canonicalize(&probe) {
            Ok(c) => break rest.iter().rev().fold(c, |acc, n| acc.join(n)),
            Err(_) => {
                let (Some(name), Some(parent)) = (probe.file_name().map(|n| n.to_os_string()), probe.parent().map(Path::to_path_buf)) else {
                    bail!("\"{}\" is not a folder in your library.", path.display());
                };
                rest.push(name);
                probe = parent;
            }
        }
    };
    if !real.starts_with(&croot) {
        bail!("\"{}\" leads outside your library (through a link), so Lumio won't change it.", path.display());
    }
    Ok(())
}

/// Refuses a path that is itself a symbolic link (deleting, renaming or moving one would
/// act on whatever it points at, or surprise the person).
pub fn not_a_link(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        bail!("\"{}\" is a link to another place, so Lumio won't change it.", path.display());
    }
    Ok(())
}

/// Did `rename` fail only because source and target are on different drives?
fn crosses_devices(e: &std::io::Error) -> bool {
    #[cfg(unix)]
    let code = 18; // EXDEV
    #[cfg(windows)]
    let code = 17; // ERROR_NOT_SAME_DEVICE
    #[cfg(not(any(unix, windows)))]
    let code = -1;
    e.raw_os_error() == Some(code)
}

fn numbered(name: &str, i: u32) -> String {
    if i < 2 {
        return name.to_string();
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    format!("{stem} ({i}){ext}")
}

/// Pick a free name like "IMG_0001 (2).jpg" (or "Trip (2)" for a folder) if the target exists.
pub fn unique_target(dir: &Path, name: &str) -> PathBuf {
    free_name(dir, name, None, |p| p.exists())
}

/// The first of "name", "name (2)", "name (3)"... in `dir` that `taken` says is free, for the
/// photo and, when it has one, for its sidecar under the matching name.
pub(crate) fn free_name(dir: &Path, name: &str, sidecar: Option<&Path>, mut taken: impl FnMut(&Path) -> bool) -> PathBuf {
    for i in 1.. {
        let c = dir.join(numbered(name, i));
        if taken(&c) {
            continue;
        }
        if let Some(s) = sidecar {
            // An identical copy of the sidecar already there (say the RAW follows its JPEG,
            // which took a copy of their shared sidecar) is fine.
            let t = sidecar_target(&c, s);
            if taken(&t) && !same_content(s, &t).unwrap_or(false) {
                continue;
            }
        }
        return c;
    }
    unreachable!()
}

/// A hidden name next to `dst` for a copy that isn't finished yet.
fn part_name(dst: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let name = dst.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    dst.with_file_name(format!(".lumio-{}-{}-{name}.part", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

fn already_there(dst: &Path) -> anyhow::Error {
    anyhow!("a file named \"{}\" is already there", dst.file_name().unwrap_or_default().to_string_lossy())
}

/// Give `from`, a file Lumio has just made, the name `to`, never replacing a file that is
/// already called that. A hard link fails if the name is taken, which rename doesn't on
/// every system; drives without hard links fall back to checking first.
pub(crate) fn place_new(from: &Path, to: &Path) -> Result<()> {
    match std::fs::hard_link(from, to) {
        Ok(()) => {
            let _ = std::fs::remove_file(from);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(already_there(to)),
        Err(_) => {
            if to.exists() {
                return Err(already_there(to));
            }
            std::fs::rename(from, to).with_context(|| format!("naming the copy {}", to.display()))
        }
    }
}

/// Are the two files byte for byte the same?
pub(crate) fn same_content(a: &Path, b: &Path) -> Result<bool> {
    use std::io::Read;
    let (ma, mb) = (std::fs::metadata(a)?, std::fs::metadata(b)?);
    if ma.len() != mb.len() {
        return Ok(false);
    }
    let mut fa = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(a)?);
    let mut fb = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(b)?);
    let mut ba = vec![0u8; 1 << 20];
    let mut bb = vec![0u8; 1 << 20];
    loop {
        let n = read_full(&mut fa, &mut ba)?;
        let m = read_full(&mut fb, &mut bb)?;
        if n != m || ba[..n] != bb[..m] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
    }
    fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            match r.read(&mut buf[n..])? {
                0 => break,
                k => n += k,
            }
        }
        Ok(n)
    }
}

/// Copy `src` to `dst`, which must not exist yet. The copy is written under a hidden name,
/// compared with the original byte for byte, and only then given its real name, so a
/// copy cut short by a drive dropping out never looks like a finished photo.
pub fn copy_verified(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Err(already_there(dst));
    }
    let part = part_name(dst);
    let res: Result<()> = (|| {
        std::fs::copy(src, &part).with_context(|| format!("copying to {}", dst.display()))?;
        if let Ok(f) = std::fs::File::open(&part) {
            let _ = f.sync_all();
        }
        if !same_content(src, &part).context("checking the copy")? {
            bail!("the copy of {} didn't match the original", src.file_name().unwrap_or_default().to_string_lossy());
        }
        // Keep the original modification time.
        if let Ok(m) = std::fs::metadata(src).and_then(|m| m.modified()) {
            if let Ok(f) = std::fs::File::options().write(true).open(&part) {
                let _ = f.set_modified(m);
            }
        }
        place_new(&part, dst)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    res
}

/// Move a file to `dst`, which must not exist yet. On the same drive this is a rename;
/// across drives it is a checked copy followed by deleting the original.
pub fn move_file(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Err(already_there(dst));
    }
    // A hard link plus removing the old name is a rename that can't replace anything.
    match std::fs::hard_link(src, dst) {
        Ok(()) => {
            if let Err(e) = std::fs::remove_file(src) {
                let _ = std::fs::remove_file(dst);
                return Err(anyhow::Error::from(e).context(format!("moving {}", src.display())));
            }
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(already_there(dst)),
        Err(_) => {}
    }
    if !dst.exists() && std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_verified(src, dst)?;
    std::fs::remove_file(src).with_context(|| format!("removing {} after copying", src.display()))?;
    Ok(())
}

/// Copies a folder tree. Only folders and ordinary files are copied: links (which could
/// pull in files from outside the library) and special files (pipes, devices, which could
/// hang the copy) are left out.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let t = dst.join(e.file_name());
        let ft = e.file_type()?;
        if ft.is_dir() {
            copy_dir_recursive(&e.path(), &t)?;
        } else if ft.is_file() {
            copy_verified(&e.path(), &t)?;
        } else {
            log::info!("not copying {} (a link or special file)", e.path().display());
        }
    }
    Ok(())
}

pub(crate) fn sidecar_target(dst_photo: &Path, sidecar: &Path) -> PathBuf {
    // Keep the sidecar naming style: "name.xmp" or "name.ext.xmp".
    let side_name = sidecar.file_name().unwrap().to_string_lossy().to_string();
    let photo_name = dst_photo.file_name().unwrap().to_string_lossy().to_string();
    if side_name.to_lowercase().ends_with(&format!(".{}.xmp", indexer::ext_of(dst_photo))) {
        dst_photo.with_file_name(format!("{photo_name}.xmp"))
    } else {
        dst_photo.with_extension(sidecar.extension().unwrap_or_default())
    }
}

/// Is a "name.xmp" sidecar also the sidecar of another photo that is staying where it is?
/// Then it has to stay too, and the photo that leaves takes a copy.
///
/// "name.xmp" belongs to a RAW file (see [`indexer::find_sidecar`]), so a JPEG staying
/// next to a departing RAW does not need it, while another RAW (or a video that keeps a
/// sidecar) does. A sidecar that a non-RAW file uses is shared with any other non-RAW
/// file of the same name, since both read it.
pub(crate) fn sidecar_shared(src: &Path, side: &Path, leaving: &HashSet<PathBuf>) -> bool {
    let side_name = side.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    let full = src.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    if side_name == format!("{full}.xmp") {
        return false; // "IMG_1.CR2.xmp" belongs to that one file.
    }
    let owner = |p: &Path| {
        let e = indexer::ext_of(p);
        metadata::is_raw(&e) || metadata::video_needs_sidecar(&e)
    };
    let src_owns = owner(src);
    let stem = src.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
    let Some(dir) = src.parent() else { return false };
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().map(|e| e.path()).any(|p| {
        p != src
            && !leaving.contains(&p)
            && p.file_stem().map(|s| s.to_string_lossy().to_lowercase()) == Some(stem.clone())
            && metadata::is_supported(&indexer::ext_of(&p))
            && (owner(&p) || !src_owns)
    })
}

/// Bring sidecar `s` along to `t`: moved, or copied when another photo still needs it.
/// If `t` already holds an identical copy there is nothing to bring.
pub(crate) fn carry_sidecar(s: &Path, t: &Path, keep_original: bool) -> Result<()> {
    if t.exists() && same_content(s, t)? {
        if !keep_original {
            std::fs::remove_file(s)?;
        }
        return Ok(());
    }
    if keep_original {
        copy_verified(s, t)
    } else {
        move_file(s, t)
    }
}

// ------------------------------------------------------------------ moving and copying photos

pub fn move_photos<H: Host + ?Sized>(h: &H, ids: &[i64], dest: &Path, copy: bool) -> Result<OpReport> {
    let items: Vec<(i64, PathBuf, Option<String>)> = ids.iter().map(|id| (*id, dest.to_path_buf(), None)).collect();
    move_photos_to(h, &items, copy)
}

struct Dest {
    root: Option<(i64, PathBuf)>,
    /// On a library drive that isn't connected: moves there wait for the drive.
    away: bool,
    problem: Option<String>,
}

/// Move (or copy) each photo to its own folder, optionally under a given name (Undo uses
/// that to put photos back exactly where they were).
pub fn move_photos_to<H: Host + ?Sized>(h: &H, items: &[(i64, PathBuf, Option<String>)], copy: bool) -> Result<OpReport> {
    let ids: Vec<i64> = items.iter().map(|(id, _, _)| *id).collect();
    let photos: HashMap<i64, db::Photo> = { db::get_photos(&h.conn().lock(), &ids)?.into_iter().map(|p| (p.id, p)).collect() };

    // Check each destination once.
    let mut dests: HashMap<PathBuf, Dest> = HashMap::new();
    for (_, dest, _) in items {
        if dests.contains_key(dest) {
            continue;
        }
        let root = { db::root_for_path(&h.conn().lock(), dest)? };
        let away = !dest.is_dir() && root.as_ref().is_some_and(|(_, r)| !indexer::reachable(r));
        let problem = if !dest.is_dir() && !away {
            Some(format!("The destination folder is not available: {}", dest.display()))
        } else if away && copy {
            Some(format!(
                "{} isn't connected. Photos can be moved there now and will follow when it's back, but copying needs the drive connected.",
                root.as_ref().map(|(_, r)| r.display().to_string()).unwrap_or_default()
            ))
        } else {
            None
        };
        dests.insert(dest.clone(), Dest { root, away, problem });
    }
    // One destination that can't be used: say so plainly instead of per photo.
    if dests.len() == 1 {
        if let Some(p) = dests.values().next().and_then(|d| d.problem.clone()) {
            bail!(p);
        }
    }

    let mut rep = OpReport { outside_library: dests.values().any(|d| d.root.is_none()), ..Default::default() };
    let total = items.len();
    let mut new_files = vec![];
    let mut touched: Vec<PathBuf> = dests.keys().cloned().collect();
    let leaving: HashSet<PathBuf> =
        if copy { HashSet::new() } else { photos.values().map(|p| PathBuf::from(&p.path)).collect() };
    for (i, (id, dest, wanted)) in items.iter().enumerate() {
        let Some(p) = photos.get(id) else { continue };
        let d = &dests[dest];
        h.progress(i, total, &format!("{} {}", if copy { "Copying" } else { "Moving" }, p.filename));
        if let Some(why) = &d.problem {
            rep.failed.push((p.filename.clone(), why.clone()));
            continue;
        }
        let name = wanted.clone().unwrap_or_else(|| p.filename.clone());
        let src = PathBuf::from(&p.path);
        if !copy && src.parent() == Some(dest.as_path()) && name == p.filename {
            continue;
        }
        // Moves that can't happen yet are made in the catalogue now and wait in
        // `pending`: when the destination's drive is away, when this photo's own drive is
        // away, or when the photo already has a move waiting.
        if !copy {
            let queue = {
                let conn = h.conn().lock();
                let waiting = crate::pending::source_of(&conn, p.id)?.is_some();
                let src_away = !src.exists()
                    && db::root_for_path(&conn, &src)?.map_or(true, |(_, root)| !indexer::reachable(&root));
                match (&d.root, d.away || src_away || waiting) {
                    (Some((root_id, _)), true) => Some(crate::pending::queue_move(&conn, p, dest, &name, *root_id)),
                    _ => None,
                }
            };
            match queue {
                Some(Ok(to)) => {
                    rep.queued += 1;
                    rep.moved.push(Moved { id: p.id, from: p.path.clone(), to: to.to_string_lossy().to_string(), in_library: true });
                    continue;
                }
                Some(Err(e)) => {
                    rep.failed.push((p.filename.clone(), e.to_string()));
                    continue;
                }
                None => {}
            }
        } else if crate::pending::source_of(&h.conn().lock(), p.id)?.is_some() {
            rep.failed.push((p.filename.clone(), "it has a move waiting for a drive; copy it once that's done".into()));
            continue;
        }
        let res: Result<PathBuf> = (|| {
            if !src.exists() {
                bail!("the file is not available (is its drive connected?)");
            }
            let side = find_sidecar(&src);
            let dst = free_name(dest, &name, side.as_deref(), |c| c.exists());
            if copy {
                copy_verified(&src, &dst)?;
                rep.copies.push((dst.clone(), src.clone()));
                if let Some(s) = &side {
                    let t = sidecar_target(&dst, s);
                    match carry_sidecar(s, &t, true) {
                        Ok(()) => rep.copies.push((t, s.clone())),
                        Err(e) => log::warn!("copied {} but not its sidecar: {e:#}", dst.display()),
                    }
                }
            } else {
                move_file(&src, &dst)?;
                if let Some(s) = &side {
                    let t = sidecar_target(&dst, s);
                    if let Err(e) = carry_sidecar(s, &t, sidecar_shared(&src, s, &leaving)) {
                        log::warn!("moved {} but not its sidecar: {e:#}", dst.display());
                    }
                }
            }
            Ok(dst)
        })();
        match res {
            Ok(dst) => {
                rep.done += 1;
                if let Some(par) = src.parent() {
                    touched.push(par.to_path_buf());
                }
                let conn = h.conn().lock();
                if copy {
                    new_files.push(dst);
                } else if let Some((root_id, _)) = &d.root {
                    let filename = dst.file_name().unwrap().to_string_lossy().to_string();
                    conn.execute(
                        "UPDATE photos SET path = ?1, folder = ?2, filename = ?3, root_id = ?4 WHERE id = ?5",
                        params![dst.to_string_lossy(), dest.to_string_lossy(), filename, root_id, p.id],
                    )?;
                    rep.moved.push(Moved { id: p.id, from: p.path.clone(), to: dst.to_string_lossy().to_string(), in_library: true });
                } else {
                    h.forget(&conn, &[p.id])?;
                    rep.moved.push(Moved { id: p.id, from: p.path.clone(), to: dst.to_string_lossy().to_string(), in_library: false });
                }
            }
            Err(e) => rep.failed.push((p.filename.clone(), format!("{e:#}"))),
        }
    }
    if !new_files.is_empty() && dests.values().any(|d| d.root.is_some()) {
        h.queue(Job::Files(new_files));
    }
    if rep.queued > 0 {
        // If both drives are in fact here (a move was waiting already), do it now.
        h.queue(Job::ApplyPending);
    }
    crate::notify::changed(&touched);
    h.progress(total, total, "");
    Ok(rep)
}

/// Move a file that is no longer in the catalogue (it was moved out of the library) back
/// into `dir` under `name`, with its sidecar, and index it again.
pub fn move_file_back<H: Host + ?Sized>(h: &H, file: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
    if !file.exists() {
        bail!("it isn't in {} any more", file.parent().map(|p| p.display().to_string()).unwrap_or_default());
    }
    if !dir.is_dir() {
        bail!("its old folder isn't available: {}", dir.display());
    }
    let side = find_sidecar(file);
    let dst = free_name(dir, name, side.as_deref(), |c| c.exists());
    move_file(file, &dst)?;
    if let Some(s) = &side {
        if let Err(e) = carry_sidecar(s, &sidecar_target(&dst, s), false) {
            log::warn!("moved {} back but not its sidecar: {e:#}", dst.display());
        }
    }
    h.queue(Job::Files(vec![dst.clone()]));
    crate::notify::changed([file, dst.as_path()]);
    Ok(dst)
}

// ------------------------------------------------------------------ folders

/// Keep moves waiting for a drive pointing at the right place after a folder is renamed
/// or moved (`src` in `pending_moves` is a real path on disk).
fn rewrite_pending(conn: &Connection, old: &str, new: &str) -> Result<()> {
    let sep = std::path::MAIN_SEPARATOR;
    let prefix = format!("{}{sep}", old.trim_end_matches(['/', '\\']));
    let mut st = conn.prepare("SELECT photo_id, src FROM pending_moves")?;
    let rows: Vec<(i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (id, src) in rows {
        if let Some(rest) = src.strip_prefix(&prefix) {
            let np = format!("{}{sep}{rest}", new.trim_end_matches(['/', '\\']));
            conn.execute("UPDATE pending_moves SET src = ?1 WHERE photo_id = ?2", params![np, id])?;
        }
    }
    Ok(())
}

pub fn move_folder<H: Host + ?Sized>(h: &H, src: &Path, dest_parent: &Path, copy: bool) -> Result<OpReport> {
    if !src.is_dir() {
        bail!("The folder is not available: {}", src.display());
    }
    if !dest_parent.is_dir() {
        bail!("The destination folder is not available: {}", dest_parent.display());
    }
    if dest_parent.starts_with(src) {
        bail!("A folder cannot be moved into itself.");
    }
    if !copy && src.parent() == Some(dest_parent) {
        return Ok(OpReport::default());
    }
    let name = src.file_name().ok_or_else(|| anyhow!("invalid folder"))?.to_string_lossy().to_string();
    let dst = unique_target(dest_parent, &name);
    let dest_root = { db::root_for_path(&h.conn().lock(), dest_parent)? };
    h.progress(0, 1, &format!("{} folder {}", if copy { "Copying" } else { "Moving" }, name));
    if copy {
        if let Err(e) = copy_dir_recursive(src, &dst) {
            let _ = std::fs::remove_dir_all(&dst); // only what this copy made: `dst` was a new name
            return Err(e);
        }
    } else if let Err(err) = std::fs::rename(src, &dst) {
        // Only a move to another drive needs copy-then-remove. Any other failure (no
        // permission, file in use…) is reported, never "fixed" by deleting the original.
        if !crosses_devices(&err) {
            return Err(anyhow::Error::from(err).context(format!("moving {}", src.display())));
        }
        if let Err(e) = copy_dir_recursive(src, &dst) {
            let _ = std::fs::remove_dir_all(&dst);
            return Err(e.context("copying the folder (the original is untouched)"));
        }
        std::fs::remove_dir_all(src).context("removing the original folder after copying")?;
    }
    {
        let conn = h.conn().lock();
        let s = src.to_string_lossy().to_string();
        let d = dst.to_string_lossy().to_string();
        if !copy {
            rewrite_pending(&conn, &s, &d)?;
            match &dest_root {
                Some((rid, _)) => {
                    db::rewrite_prefix(&conn, &s, &d, Some(*rid))?;
                }
                None => {
                    let mut args = vec![];
                    let clause = db::folder_clause(&s, true, &mut args);
                    let mut stmt = conn.prepare(&format!("SELECT id FROM photos WHERE {clause}"))?;
                    let ids: Vec<i64> = stmt
                        .query_map(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    h.forget(&conn, &ids)?;
                }
            }
        }
    }
    crate::notify::changed([src.parent().unwrap_or(src), dest_parent, dst.as_path()]);
    if dest_root.is_some() {
        h.queue(Job::ScanFolder(dst.clone()));
    }
    h.progress(1, 1, "");
    Ok(OpReport { done: 1, outside_library: dest_root.is_none(), folder: Some(dst), ..Default::default() })
}

pub(crate) fn check_name(name: &str) -> Result<()> {
    let n = name.trim();
    if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        bail!("\"{name}\" is not a valid name.");
    }
    Ok(())
}

pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf> {
    check_name(name)?;
    let p = parent.join(name.trim());
    if p.exists() {
        bail!("A folder named \"{}\" already exists here.", name.trim());
    }
    std::fs::create_dir(&p)?;
    crate::notify::changed([parent, p.as_path()]);
    Ok(p)
}

pub fn rename_folder<H: Host + ?Sized>(h: &H, path: &Path, new_name: &str) -> Result<PathBuf> {
    check_name(new_name)?;
    if !path.is_dir() {
        bail!("The folder is not available: {}", path.display());
    }
    let dst = path.with_file_name(new_name.trim());
    if dst == path {
        return Ok(dst);
    }
    if same_place(&dst, path) {
        // Only the case changes ("trip" -> "Trip"). Drives that ignore case see the new
        // name as taken by the folder itself, so go through a temporary name.
        let tmp = path.with_file_name(format!(".lumio-rename-{}.tmp", std::process::id()));
        std::fs::rename(path, &tmp)?;
        if let Err(e) = std::fs::rename(&tmp, &dst) {
            let _ = std::fs::rename(&tmp, path);
            return Err(e.into());
        }
    } else {
        if dst.exists() {
            bail!("A folder named \"{}\" already exists.", new_name.trim());
        }
        std::fs::rename(path, &dst)?;
    }
    crate::notify::changed([dst.as_path()]);
    let conn = h.conn().lock();
    rewrite_pending(&conn, &path.to_string_lossy(), &dst.to_string_lossy())?;
    db::rewrite_prefix(&conn, &path.to_string_lossy(), &dst.to_string_lossy(), None)?;
    // If it was a library root, update that too.
    conn.execute(
        "UPDATE roots SET path = ?1, name = ?2 WHERE path = ?3",
        params![dst.to_string_lossy(), new_name.trim(), path.to_string_lossy()],
    )?;
    Ok(dst)
}

// ------------------------------------------------------------------ deleting

/// Move photos to the system Trash / Recycle Bin (or delete permanently).
pub fn delete_photos<H: Host + ?Sized>(h: &H, ids: &[i64], permanent: bool) -> Result<OpReport> {
    let photos = { db::get_photos(&h.conn().lock(), ids)? };
    let leaving: HashSet<PathBuf> = photos.iter().map(|p| PathBuf::from(&p.path)).collect();
    let mut rep = OpReport::default();
    for p in photos {
        let src = PathBuf::from(&p.path);
        let mut targets = vec![src.clone()];
        // A sidecar the RAW (or JPEG) left behind still needs stays where it is.
        if let Some(s) = find_sidecar(&src) {
            if !sidecar_shared(&src, &s, &leaving) {
                targets.push(s);
            }
        }
        let r: Result<()> = if !src.exists() {
            Err(anyhow!("the file is not available (is its drive connected?)"))
        } else if permanent {
            targets.iter().try_for_each(|t| std::fs::remove_file(t).map_err(Into::into))
        } else {
            trash::delete_all(&targets).map_err(|e| anyhow!("could not move to Trash: {e}"))
        };
        match r {
            Ok(()) => {
                rep.done += 1;
                crate::notify::changed([src.as_path()]);
                h.forget(&h.conn().lock(), &[p.id])?;
            }
            Err(e) => {
                rep.failed.push((p.filename.clone(), e.to_string()));
                rep.failed_ids.push(p.id);
            }
        }
    }
    Ok(rep)
}

pub fn delete_folder<H: Host + ?Sized>(h: &H, path: &Path, permanent: bool) -> Result<()> {
    if !path.is_dir() {
        bail!("The folder is not available.");
    }
    if permanent {
        std::fs::remove_dir_all(path)?;
    } else {
        trash::delete(path).map_err(|e| anyhow!("could not move to Trash: {e}"))?;
    }
    if let Some(parent) = path.parent() {
        crate::notify::changed([parent]);
    }
    h.queue(Job::Removed(vec![path.to_path_buf()]));
    Ok(())
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
pub(crate) mod testing {
    //! A stand-in for the app, a throwaway folder and a catalogue, for tests that work on
    //! real files.
    use super::*;

    pub struct TestHost {
        pub conn: Mutex<Connection>,
        pub jobs: Mutex<Vec<Job>>,
    }

    impl Host for TestHost {
        fn conn(&self) -> &Mutex<Connection> {
            &self.conn
        }
        fn queue(&self, job: Job) {
            self.jobs.lock().push(job);
        }
        fn forget(&self, conn: &Connection, ids: &[i64]) -> Result<()> {
            for id in ids {
                conn.execute("DELETE FROM photos WHERE id = ?1", [id])?;
            }
            Ok(())
        }
    }

    /// A temporary folder, removed when dropped.
    pub struct Tmp(pub PathBuf);
    impl Tmp {
        pub fn new(tag: &str) -> Tmp {
            // A fresh folder with an unguessable name (removed by Drop below).
            Tmp(tempfile::Builder::new().prefix(&format!("lumio-{tag}-")).tempdir().unwrap().keep())
        }
        /// `t.join("a/b.jpg")` with the platform's separator, so paths compare equal to
        /// the ones Lumio builds (on Windows "a\\b.jpg", not "a/b.jpg").
        pub fn join(&self, s: &str) -> PathBuf {
            s.split('/').filter(|c| !c.is_empty()).fold(self.0.clone(), |p, c| p.join(c))
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub fn host() -> TestHost {
        let conn = Connection::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        TestHost { conn: Mutex::new(conn), jobs: Mutex::new(vec![]) }
    }

    impl TestHost {
        pub fn root(&self, path: &Path) -> i64 {
            let c = self.conn.lock();
            c.execute("INSERT INTO roots (path, name, added_at) VALUES (?1, 'r', 0)", [path.to_string_lossy()]).unwrap();
            c.last_insert_rowid()
        }
        /// Write a file (its content is its own name unless given) and catalogue it unless it's a sidecar.
        pub fn file(&self, root: i64, path: &Path, content: Option<&str>) -> i64 {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            std::fs::write(path, content.unwrap_or(&name)).unwrap();
            if indexer::ext_of(path) == "xmp" {
                return 0;
            }
            self.photo(root, path)
        }
        /// Catalogue a photo without making the file.
        pub fn photo(&self, root: i64, path: &Path) -> i64 {
            let c = self.conn.lock();
            let size = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(5);
            c.execute(
                "INSERT INTO photos (root_id, path, folder, filename, ext, size, rating) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 4)",
                params![
                    root,
                    path.to_string_lossy(),
                    path.parent().unwrap().to_string_lossy(),
                    path.file_name().unwrap().to_string_lossy(),
                    indexer::ext_of(path),
                    size
                ],
            )
            .unwrap();
            c.last_insert_rowid()
        }
        pub fn path_of(&self, id: i64) -> Option<String> {
            db::get_photo(&self.conn.lock(), id).unwrap().map(|p| p.path)
        }
        pub fn rating(&self, id: i64) -> i64 {
            db::get_photo(&self.conn.lock(), id).unwrap().unwrap().rating
        }
    }

    pub fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = match std::fs::read_dir(dir) {
            // Finder leaves a .DS_Store behind when it puts files in the Trash.
            Ok(rd) => rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n != ".DS_Store").collect(),
            Err(_) => vec![],
        };
        v.sort();
        v
    }

    pub fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[cfg(unix)]
    #[test]
    fn links_out_of_a_root_are_refused() {
        let lib = Tmp::new("lib");
        let outside = Tmp::new("outside");
        std::fs::write(outside.join("secret.txt"), "x").unwrap();
        std::fs::create_dir_all(lib.join("Trip")).unwrap();
        std::os::unix::fs::symlink(&outside.0, lib.join("escape_link")).unwrap();
        assert!(inside_root(&lib.0, &lib.join("Trip")).is_ok());
        assert!(inside_root(&lib.0, &lib.join("Trip/New folder")).is_ok(), "a folder about to be created");
        assert!(inside_root(&lib.0, &lib.join("escape_link")).is_err());
        assert!(inside_root(&lib.0, &lib.join("escape_link/sub/new")).is_err());
        assert!(not_a_link(&lib.join("escape_link")).is_err());
        assert!(not_a_link(&lib.join("Trip")).is_ok());
        // Unplugged drive: nothing to follow.
        assert!(inside_root(Path::new("/nonexistent-lumio-root"), Path::new("/nonexistent-lumio-root/a")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn copying_a_folder_skips_links_and_special_files() {
        let t = Tmp::new("copylinks");
        let outside = Tmp::new("copyout");
        std::fs::write(outside.join("private.txt"), "secret").unwrap();
        std::fs::create_dir_all(t.join("src/sub")).unwrap();
        std::fs::write(t.join("src/a.jpg"), "a").unwrap();
        std::fs::write(t.join("src/sub/b.jpg"), "b").unwrap();
        std::os::unix::fs::symlink(outside.join("private.txt"), t.join("src/link.txt")).unwrap();
        std::os::unix::fs::symlink(&outside.0, t.join("src/dirlink")).unwrap();
        let fifo = t.join("src/pipe");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        copy_dir_recursive(&t.join("src"), &t.join("dst")).unwrap();
        assert_eq!(names(&t.join("dst")), vec!["a.jpg", "sub"]);
        assert_eq!(names(&t.join("dst/sub")), vec!["b.jpg"]);
    }

    #[test]
    fn same_place_follows_the_drive() {
        let t = Tmp::new("case");
        std::fs::write(t.join("IMG_1.jpg"), "a").unwrap();
        assert!(same_place(&t.join("IMG_1.jpg"), &t.join("IMG_1.jpg")));
        assert!(!same_place(&t.join("IMG_1.jpg"), &t.join("IMG_2.jpg")));
        let upper = t.join("IMG_1.JPG");
        if upper.exists() {
            // This drive ignores case: both names are the one file.
            assert!(same_place(&t.join("IMG_1.jpg"), &upper));
        } else {
            std::fs::write(&upper, "b").unwrap();
            // Case-sensitive drive: two different files.
            assert!(!same_place(&t.join("IMG_1.jpg"), &upper));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_folder_move_never_deletes_the_original() {
        use std::os::unix::fs::PermissionsExt;
        let h = host();
        let t = Tmp::new("mvfail");
        let r = h.root(&t.0);
        h.file(r, &t.join("A/x.jpg"), None);
        std::fs::create_dir_all(t.join("B")).unwrap();
        // A destination we may not write into: rename fails (not a cross-device error).
        std::fs::set_permissions(t.join("B"), std::fs::Permissions::from_mode(0o555)).unwrap();
        let res = move_folder(&h, &t.join("A"), &t.join("B"), false);
        std::fs::set_permissions(t.join("B"), std::fs::Permissions::from_mode(0o755)).unwrap();
        if nix_is_root() {
            return; // root ignores permissions, so the move just works
        }
        assert!(res.is_err());
        assert!(t.join("A/x.jpg").exists());
    }

    #[cfg(unix)]
    fn nix_is_root() -> bool {
        std::process::Command::new("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0").unwrap_or(false)
    }

    #[test]
    fn move_file_never_replaces_an_existing_file() {
        let t = Tmp::new("fo");
        let (a, b) = (t.join("a.jpg"), t.join("b.jpg"));
        std::fs::write(&a, "A").unwrap();
        std::fs::write(&b, "B").unwrap();
        assert!(move_file(&a, &b).is_err());
        assert_eq!((read(&a), read(&b)), ("A".into(), "B".into()));
        assert!(copy_verified(&a, &b).is_err());
        assert_eq!(read(&b), "B", "a refused copy must not touch the file that was there");
        // Into a free name it works, and leaves no temporary files behind.
        move_file(&a, &t.join("c.jpg")).unwrap();
        assert_eq!(names(&t.0), ["b.jpg", "c.jpg"]);
    }

    #[test]
    fn copies_are_checked_and_keep_the_date() {
        let t = Tmp::new("fo");
        let a = t.join("a.jpg");
        std::fs::write(&a, vec![7u8; 3_000_000]).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
        std::fs::File::options().write(true).open(&a).unwrap().set_modified(old).unwrap();
        let c = t.join("copy.jpg");
        copy_verified(&a, &c).unwrap();
        assert!(same_content(&a, &c).unwrap());
        assert_eq!(std::fs::metadata(&c).unwrap().modified().unwrap(), old);
        assert_eq!(names(&t.0), ["a.jpg", "copy.jpg"]);
        std::fs::write(&c, vec![7u8; 2_999_999]).unwrap();
        assert!(!same_content(&a, &c).unwrap());
        let mut d = vec![7u8; 3_000_000];
        d[2_500_000] = 8;
        std::fs::write(&c, d).unwrap();
        assert!(!same_content(&a, &c).unwrap());
    }

    #[test]
    fn copying_from_a_missing_file_leaves_nothing_behind() {
        let t = Tmp::new("fo");
        assert!(copy_verified(&t.join("gone.jpg"), &t.join("new.jpg")).is_err());
        assert!(names(&t.0).is_empty());
    }

    #[test]
    fn moves_photo_sidecar_and_catalogue() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("in/IMG_1.CR2"), None);
        h.file(r, &t.join("in/IMG_1.xmp"), None);
        std::fs::create_dir_all(t.join("out")).unwrap();
        let rep = move_photos(&h, &[id], &t.join("out"), false).unwrap();
        assert_eq!((rep.done, rep.failed.len()), (1, 0));
        assert_eq!(names(&t.join("out")), ["IMG_1.CR2", "IMG_1.xmp"]);
        assert!(names(&t.join("in")).is_empty());
        assert_eq!(h.path_of(id).unwrap(), t.join("out/IMG_1.CR2").to_string_lossy());
        assert_eq!(h.rating(id), 4, "the catalogue entry (and its rating) moves with the file");
        assert_eq!(rep.moved.len(), 1);
        assert_eq!(rep.moved[0].from, t.join("in/IMG_1.CR2").to_string_lossy());
    }

    #[test]
    fn moving_never_overwrites_a_photo_or_a_sidecar() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("in/IMG_1.jpg"), Some("mine"));
        h.file(r, &t.join("in/IMG_1.xmp"), Some("my sidecar"));
        // The destination has a different IMG_1.jpg, and an IMG_1.xmp that belongs to an IMG_1.CR2.
        h.file(r, &t.join("out/IMG_1.jpg"), Some("theirs"));
        h.file(r, &t.join("out/IMG_1.CR2"), Some("raw"));
        h.file(r, &t.join("out/IMG_1.xmp"), Some("their sidecar"));
        let rep = move_photos(&h, &[id], &t.join("out"), false).unwrap();
        assert_eq!(rep.done, 1, "{:?}", rep.failed);
        let out = t.join("out");
        assert_eq!(read(&out.join("IMG_1.jpg")), "theirs");
        assert_eq!(read(&out.join("IMG_1.xmp")), "their sidecar");
        assert_eq!(read(&out.join("IMG_1 (2).jpg")), "mine");
        assert_eq!(read(&out.join("IMG_1 (2).xmp")), "my sidecar");
    }

    #[test]
    fn sidecar_name_taken_even_when_photo_name_is_free() {
        // Only the sidecar's name is taken: the photo must not take a name whose sidecar
        // would land on someone else's.
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("in/IMG_1.jpg"), Some("mine"));
        h.file(r, &t.join("in/IMG_1.xmp"), Some("my sidecar"));
        h.file(r, &t.join("out/IMG_1.CR2"), Some("raw"));
        h.file(r, &t.join("out/IMG_1.xmp"), Some("their sidecar"));
        let rep = move_photos(&h, &[id], &t.join("out"), false).unwrap();
        assert_eq!(rep.done, 1);
        assert_eq!(read(&t.join("out/IMG_1.xmp")), "their sidecar");
        assert_eq!(read(&t.join("out/IMG_1 (2).xmp")), "my sidecar");
        assert_eq!(read(&t.join("out/IMG_1 (2).jpg")), "mine");
    }

    #[test]
    fn moving_the_raw_of_a_raw_jpeg_pair_takes_its_sidecar() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let raw = h.file(r, &t.join("in/IMG_1.CR2"), None);
        h.file(r, &t.join("in/IMG_1.JPG"), None);
        h.file(r, &t.join("in/IMG_1.xmp"), Some("edits"));
        std::fs::create_dir_all(t.join("out")).unwrap();
        move_photos(&h, &[raw], &t.join("out"), false).unwrap();
        // No copy is left behind: the JPEG would otherwise start showing the RAW's rating.
        assert_eq!(names(&t.join("in")), ["IMG_1.JPG"]);
        assert_eq!(names(&t.join("out")), ["IMG_1.CR2", "IMG_1.xmp"]);
    }

    #[test]
    fn moving_the_jpeg_of_a_raw_jpeg_pair_leaves_the_raw_sidecar() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let raw = h.file(r, &t.join("in/IMG_1.CR2"), None);
        let jpg = h.file(r, &t.join("in/IMG_1.JPG"), None);
        h.file(r, &t.join("in/IMG_1.xmp"), Some("edits"));
        std::fs::create_dir_all(t.join("out")).unwrap();
        move_photos(&h, &[jpg], &t.join("out"), false).unwrap();
        assert_eq!(names(&t.join("in")), ["IMG_1.CR2", "IMG_1.xmp"], "the RAW keeps its sidecar");
        // "IMG_1.xmp" is the RAW's (Lightroom's naming), so the JPEG leaves without it.
        assert_eq!(names(&t.join("out")), ["IMG_1.JPG"]);
        // Moving the RAW after it brings the sidecar along.
        move_photos(&h, &[raw], &t.join("out"), false).unwrap();
        assert!(names(&t.join("in")).is_empty());
        assert_eq!(names(&t.join("out")), ["IMG_1.CR2", "IMG_1.JPG", "IMG_1.xmp"]);
        assert_eq!(read(&t.join("out/IMG_1.xmp")), "edits");
    }

    #[test]
    fn moving_the_whole_pair_moves_the_sidecar() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let raw = h.file(r, &t.join("in/IMG_1.CR2"), None);
        let jpg = h.file(r, &t.join("in/IMG_1.JPG"), None);
        h.file(r, &t.join("in/IMG_1.xmp"), None);
        std::fs::create_dir_all(t.join("out")).unwrap();
        let rep = move_photos(&h, &[raw, jpg], &t.join("out"), false).unwrap();
        assert_eq!(rep.done, 2);
        assert!(names(&t.join("in")).is_empty());
        assert_eq!(names(&t.join("out")), ["IMG_1.CR2", "IMG_1.JPG", "IMG_1.xmp"]);
    }

    #[test]
    fn two_photos_with_the_same_name_from_different_folders() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let a = h.file(r, &t.join("a/IMG_1.jpg"), Some("a"));
        let b = h.file(r, &t.join("b/IMG_1.jpg"), Some("b"));
        std::fs::create_dir_all(t.join("out")).unwrap();
        let rep = move_photos(&h, &[a, b], &t.join("out"), false).unwrap();
        assert_eq!(rep.done, 2);
        assert_eq!(names(&t.join("out")), ["IMG_1 (2).jpg", "IMG_1.jpg"]);
        assert_eq!(read(&t.join("out/IMG_1 (2).jpg")), "b");
        assert_eq!(h.path_of(b).unwrap(), t.join("out/IMG_1 (2).jpg").to_string_lossy());
    }

    #[test]
    fn copy_keeps_the_original_and_queues_indexing() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("in/IMG_1.jpg"), None);
        h.file(r, &t.join("in/IMG_1.xmp"), None);
        std::fs::create_dir_all(t.join("out")).unwrap();
        let rep = move_photos(&h, &[id], &t.join("out"), true).unwrap();
        assert_eq!(rep.done, 1);
        assert_eq!(names(&t.join("in")), ["IMG_1.jpg", "IMG_1.xmp"]);
        assert_eq!(names(&t.join("out")), ["IMG_1.jpg", "IMG_1.xmp"]);
        assert_eq!(rep.copies.len(), 2);
        assert!(h.jobs.lock().iter().any(|j| matches!(j, Job::Files(f) if f.len() == 1)));
        // Copying into the same folder makes a numbered copy.
        let rep = move_photos(&h, &[id], &t.join("in"), true).unwrap();
        assert_eq!(rep.done, 1);
        assert_eq!(names(&t.join("in")), ["IMG_1 (2).jpg", "IMG_1 (2).xmp", "IMG_1.jpg", "IMG_1.xmp"]);
    }

    #[test]
    fn missing_file_on_a_connected_drive_fails_without_changes() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        std::fs::create_dir_all(t.join("in")).unwrap();
        std::fs::create_dir_all(t.join("out")).unwrap();
        let id = h.photo(r, &t.join("in/gone.jpg"));
        let rep = move_photos(&h, &[id], &t.join("out"), false).unwrap();
        assert_eq!((rep.done, rep.queued, rep.failed.len()), (0, 0, 1));
        assert_eq!(h.path_of(id).unwrap(), t.join("in/gone.jpg").to_string_lossy());
    }

    #[test]
    fn unavailable_destination_is_refused() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.join("lib"));
        let id = h.file(r, &t.join("lib/a.jpg"), None);
        // Not a library folder, and not there.
        assert!(move_photos(&h, &[id], &t.join("nowhere"), false).is_err());
        assert!(t.join("lib/a.jpg").exists());
    }

    #[test]
    fn moving_to_a_drive_that_is_away_waits_and_copying_is_refused() {
        let t = Tmp::new("fo");
        let h = host();
        let ra = h.root(&t.join("a"));
        h.root(&t.join("b")); // drive b isn't connected: its folder doesn't exist
        let id = h.file(ra, &t.join("a/IMG_1.jpg"), None);
        assert!(move_photos(&h, &[id], &t.join("b/Trip"), true).is_err());
        let rep = move_photos(&h, &[id], &t.join("b/Trip"), false).unwrap();
        assert_eq!((rep.done, rep.queued), (0, 1));
        assert!(t.join("a/IMG_1.jpg").exists(), "the file waits where it is");
        assert_eq!(h.path_of(id).unwrap(), t.join("b/Trip/IMG_1.jpg").to_string_lossy());
        assert!(h.jobs.lock().iter().any(|j| matches!(j, Job::ApplyPending)));
        // The drive comes back: the move happens.
        std::fs::create_dir_all(t.join("b")).unwrap();
        let rep = crate::pending::apply_in(&h.conn.lock()).unwrap();
        assert_eq!(rep.done, 1);
        assert!(t.join("b/Trip/IMG_1.jpg").exists());
        assert!(!t.join("a/IMG_1.jpg").exists());
    }

    #[test]
    fn moving_out_of_the_library_forgets_the_photo() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.join("lib"));
        let id = h.file(r, &t.join("lib/a.jpg"), None);
        std::fs::create_dir_all(t.join("elsewhere")).unwrap();
        let rep = move_photos(&h, &[id], &t.join("elsewhere"), false).unwrap();
        assert!(rep.outside_library);
        assert!(h.path_of(id).is_none());
        assert!(!rep.moved[0].in_library);
        // And back again.
        let back = move_file_back(&h, Path::new(&rep.moved[0].to), &t.join("lib"), "a.jpg").unwrap();
        assert_eq!(back, t.join("lib/a.jpg"));
    }

    #[test]
    fn deleting_one_of_a_pair_keeps_the_shared_sidecar() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        h.file(r, &t.join("IMG_1.CR2"), None);
        let jpg = h.file(r, &t.join("IMG_1.JPG"), None);
        let other = h.file(r, &t.join("IMG_2.JPG"), None);
        h.file(r, &t.join("IMG_1.xmp"), None);
        h.file(r, &t.join("IMG_2.xmp"), None);
        let rep = delete_photos(&h, &[jpg, other], true).unwrap();
        assert_eq!(rep.done, 2);
        assert_eq!(names(&t.0), ["IMG_1.CR2", "IMG_1.xmp"]);
    }

    #[test]
    fn folder_rename_keeps_photos_and_waiting_moves() {
        let t = Tmp::new("fo");
        let h = host();
        let ra = h.root(&t.join("a"));
        h.root(&t.join("b")); // away
        let id = h.file(ra, &t.join("a/Trip/IMG_1.jpg"), None);
        let keep = h.file(ra, &t.join("a/Trip/IMG_2.jpg"), None);
        let lookalike = h.file(ra, &t.join("a/Trip 2/IMG_3.jpg"), None);
        move_photos(&h, &[id], &t.join("b/Out"), false).unwrap();
        let dst = rename_folder(&h, &t.join("a/Trip"), "Iceland").unwrap();
        assert_eq!(dst, t.join("a/Iceland"));
        assert_eq!(h.path_of(keep).unwrap(), t.join("a/Iceland/IMG_2.jpg").to_string_lossy());
        assert_eq!(h.path_of(lookalike).unwrap(), t.join("a/Trip 2/IMG_3.jpg").to_string_lossy(), "a folder whose name only starts the same is left alone");
        // The waiting move now starts from the renamed folder, so it still works.
        assert_eq!(crate::pending::source_of(&h.conn.lock(), id).unwrap().unwrap(), t.join("a/Iceland/IMG_1.jpg").to_string_lossy());
        std::fs::create_dir_all(t.join("b")).unwrap();
        assert_eq!(crate::pending::apply_in(&h.conn.lock()).unwrap().done, 1);
        assert!(t.join("b/Out/IMG_1.jpg").exists());
    }

    #[test]
    fn folder_rename_refuses_a_taken_name_but_allows_a_case_change() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("trip/IMG_1.jpg"), None);
        std::fs::create_dir_all(t.join("other")).unwrap();
        assert!(rename_folder(&h, &t.join("trip"), "other").is_err());
        assert!(rename_folder(&h, &t.join("trip"), "a/b").is_err());
        let dst = rename_folder(&h, &t.join("trip"), "Trip").unwrap();
        assert_eq!(names(&t.0), ["Trip", "other"]);
        assert_eq!(h.path_of(id).unwrap(), dst.join("IMG_1.jpg").to_string_lossy());
    }

    #[test]
    fn folder_move_and_copy() {
        let t = Tmp::new("fo");
        let h = host();
        let r = h.root(&t.0);
        let id = h.file(r, &t.join("Trip/IMG_1.jpg"), None);
        h.file(r, &t.join("Trip/sub/IMG_2.jpg"), None);
        std::fs::create_dir_all(t.join("Travel/Trip")).unwrap(); // the name is taken there
        assert!(move_folder(&h, &t.join("Trip"), &t.join("Trip/sub"), false).is_err(), "not into itself");
        let rep = move_folder(&h, &t.join("Trip"), &t.join("Travel"), false).unwrap();
        let dst = rep.folder.unwrap();
        assert_eq!(dst, t.join("Travel/Trip (2)"));
        assert_eq!(h.path_of(id).unwrap(), dst.join("IMG_1.jpg").to_string_lossy());
        assert!(!t.join("Trip").exists());
        let rep = move_folder(&h, &dst, &t.0, true).unwrap();
        let copy = rep.folder.unwrap();
        assert_eq!(names(&copy), ["IMG_1.jpg", "sub"]);
        assert_eq!(names(&dst), ["IMG_1.jpg", "sub"], "the original stays");
    }
}
