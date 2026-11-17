//! The catalogue: a small SQLite database that remembers every photo, its
//! metadata and its thumbnail, so folders on unplugged drives can still be browsed.

use anyhow::Result;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;",
    )?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS roots (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            added_at INTEGER NOT NULL,
            last_scan INTEGER
        );
        CREATE TABLE IF NOT EXISTS photos (
            id INTEGER PRIMARY KEY,
            root_id INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
            path TEXT NOT NULL UNIQUE,
            folder TEXT NOT NULL,
            filename TEXT NOT NULL,
            ext TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            mtime INTEGER NOT NULL DEFAULT 0,
            width INTEGER, height INTEGER, orientation INTEGER,
            taken_at TEXT,
            rating INTEGER NOT NULL DEFAULT 0,
            label TEXT,
            title TEXT, caption TEXT, keywords TEXT,
            creator TEXT, copyright TEXT,
            lat REAL, lon REAL, alt REAL,
            place TEXT,
            make TEXT, model TEXT, lens TEXT,
            iso INTEGER, fnumber REAL, exposure REAL, focal REAL,
            thumb INTEGER NOT NULL DEFAULT 0,
            indexed_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS photos_folder ON photos(folder);
        CREATE INDEX IF NOT EXISTS photos_root ON photos(root_id);
        CREATE INDEX IF NOT EXISTS photos_taken ON photos(taken_at);
        CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        "#,
    )?;
    // Added in 0.3: failed thumbnail attempts, and stacks (groups of shots).
    add_column(conn, "photos", "thumb_tries", "INTEGER NOT NULL DEFAULT 0")?;
    add_column(conn, "photos", "stack_id", "INTEGER")?;
    add_column(conn, "photos", "stack_top", "INTEGER NOT NULL DEFAULT 0")?;
    conn.execute_batch("CREATE INDEX IF NOT EXISTS photos_stack ON photos(stack_id);")?;
    Ok(())
}

/// Keep stacks consistent after photos leave the catalogue: a stack needs two
/// photos, and exactly one cover.
pub fn tidy_stacks(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "UPDATE photos SET stack_id = NULL, stack_top = 0 WHERE stack_id IN
            (SELECT stack_id FROM photos WHERE stack_id IS NOT NULL GROUP BY stack_id HAVING COUNT(*) < 2);
         UPDATE photos SET stack_top = 1 WHERE id IN
            (SELECT MIN(id) FROM photos WHERE stack_id IS NOT NULL GROUP BY stack_id HAVING SUM(stack_top) = 0);",
    )?;
    Ok(())
}

fn add_column(conn: &Connection, table: &str, col: &str, decl: &str) -> Result<()> {
    let exists: bool = conn
        .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?
        .exists([col])?;
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl};"))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Root {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub online: bool,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Photo {
    pub id: i64,
    pub root_id: i64,
    pub path: String,
    pub folder: String,
    pub filename: String,
    pub ext: String,
    pub size: i64,
    pub mtime: i64,
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
    pub thumb: bool,
}

pub const PHOTO_COLS: &str = "id, root_id, path, folder, filename, ext, size, mtime, width, height, orientation, taken_at, rating, label, title, caption, keywords, creator, copyright, lat, lon, alt, place, make, model, lens, iso, fnumber, exposure, focal, thumb";

pub fn photo_from_row(r: &Row) -> rusqlite::Result<Photo> {
    let kw: Option<String> = r.get(16)?;
    Ok(Photo {
        id: r.get(0)?,
        root_id: r.get(1)?,
        path: r.get(2)?,
        folder: r.get(3)?,
        filename: r.get(4)?,
        ext: r.get(5)?,
        size: r.get(6)?,
        mtime: r.get(7)?,
        width: r.get(8)?,
        height: r.get(9)?,
        orientation: r.get(10)?,
        taken_at: r.get(11)?,
        rating: r.get(12)?,
        label: r.get(13)?,
        title: r.get(14)?,
        caption: r.get(15)?,
        keywords: decode_keywords(kw.as_deref()),
        creator: r.get(17)?,
        copyright: r.get(18)?,
        lat: r.get(19)?,
        lon: r.get(20)?,
        alt: r.get(21)?,
        place: r.get(22)?,
        make: r.get(23)?,
        model: r.get(24)?,
        lens: r.get(25)?,
        iso: r.get(26)?,
        fnumber: r.get(27)?,
        exposure: r.get(28)?,
        focal: r.get(29)?,
        thumb: r.get::<_, i64>(30)? != 0,
    })
}

/// Keywords are stored as `|kw1|kw2|` so that `LIKE '%|kw|%'` finds exact keywords.
pub fn encode_keywords(k: &[String]) -> Option<String> {
    let clean: Vec<String> = k
        .iter()
        .map(|s| s.trim().replace('|', "/"))
        .filter(|s| !s.is_empty())
        .collect();
    if clean.is_empty() {
        None
    } else {
        Some(format!("|{}|", clean.join("|")))
    }
}

pub fn decode_keywords(s: Option<&str>) -> Vec<String> {
    s.map(|s| s.split('|').filter(|x| !x.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// A lighter record for the grid and the map.
#[derive(Debug, Clone, Serialize)]
pub struct PhotoLite {
    pub id: i64,
    pub path: String,
    pub filename: String,
    pub folder: String,
    pub ext: String,
    pub rating: i64,
    pub label: Option<String>,
    pub taken_at: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub thumb: bool,
    pub mtime: i64,
    pub root_id: i64,
    pub orientation: Option<i64>,
    /// Photos grouped together share a stack id; one of them is the cover (`stack_top`).
    pub stack_id: Option<i64>,
    pub stack_top: bool,
    pub stack_count: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Query {
    /// "all", "folder", "ids", "recent", "geotagged", "untagged"
    pub scope: String,
    pub folder: Option<String>,
    pub recursive: bool,
    pub ids: Option<Vec<i64>>,
    pub text: Option<String>,
    pub min_rating: Option<i64>,
    pub label: Option<String>,
    pub sort: Option<String>,
    pub desc: bool,
}

pub fn folder_clause(folder: &str, recursive: bool, args: &mut Vec<rusqlite::types::Value>) -> String {
    let f = folder.trim_end_matches(['/', '\\']).to_string();
    let f = if f.is_empty() { folder.to_string() } else { f };
    if recursive {
        let prefix = format!("{}{}", f, MAIN_SEPARATOR);
        args.push(f.clone().into());
        args.push((prefix.chars().count() as i64).into());
        args.push(prefix.into());
        let n = args.len();
        format!("(folder = ?{} OR substr(folder, 1, ?{}) = ?{})", n - 2, n - 1, n)
    } else {
        args.push(f.into());
        format!("folder = ?{}", args.len())
    }
}

pub fn query_photos(conn: &Connection, q: &Query) -> Result<Vec<PhotoLite>> {
    let mut wh: Vec<String> = vec![];
    let mut args: Vec<rusqlite::types::Value> = vec![];
    match q.scope.as_str() {
        "folder" => {
            if let Some(f) = &q.folder {
                wh.push(folder_clause(f, q.recursive, &mut args));
            }
        }
        "ids" => {
            let ids = q.ids.clone().unwrap_or_default();
            if ids.is_empty() {
                return Ok(vec![]);
            }
            let list: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
            wh.push(format!("id IN ({})", list.join(",")));
        }
        "recent" => {
            let cutoff = chrono::Utc::now().timestamp_millis() - 30 * 24 * 3600 * 1000;
            args.push(cutoff.into());
            wh.push(format!("indexed_at >= ?{}", args.len()));
        }
        "geotagged" => wh.push("lat IS NOT NULL AND lon IS NOT NULL".into()),
        "nogps" => wh.push("(lat IS NULL OR lon IS NULL)".into()),
        "rated" => wh.push("rating > 0".into()),
        _ => {}
    }
    if let Some(t) = q.text.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        for word in t.split_whitespace() {
            args.push(format!("%{}%", word.to_lowercase()).into());
            let n = args.len();
            wh.push(format!(
                "(lower(filename) LIKE ?{n} OR lower(ifnull(title,'')) LIKE ?{n} OR lower(ifnull(caption,'')) LIKE ?{n} OR lower(ifnull(keywords,'')) LIKE ?{n} OR lower(ifnull(place,'')) LIKE ?{n} OR lower(ifnull(model,'')) LIKE ?{n} OR lower(ifnull(lens,'')) LIKE ?{n} OR lower(folder) LIKE ?{n})"
            ));
        }
    }
    if let Some(r) = q.min_rating.filter(|r| *r > 0) {
        args.push(r.into());
        wh.push(format!("rating >= ?{}", args.len()));
    }
    if let Some(l) = q.label.as_ref().filter(|l| !l.is_empty()) {
        args.push(l.clone().into());
        wh.push(format!("label = ?{}", args.len()));
    }
    let dir = if q.desc { "DESC" } else { "ASC" };
    let order = match q.sort.as_deref() {
        Some("name") => format!("filename COLLATE NOCASE {dir}"),
        Some("rating") => format!("rating {dir}, coalesce(taken_at, '') {dir}"),
        Some("modified") => format!("mtime {dir}"),
        Some("size") => format!("size {dir}"),
        _ => format!("coalesce(taken_at, datetime(mtime/1000, 'unixepoch')) {dir}, filename {dir}"),
    };
    let sql = format!(
        "SELECT id, path, filename, folder, ext, rating, label, taken_at, width, height, lat, lon, thumb, mtime, root_id, orientation, stack_id, stack_top,
                CASE WHEN stack_id IS NULL THEN 0 ELSE (SELECT COUNT(*) FROM photos s WHERE s.stack_id = photos.stack_id) END
         FROM photos {} ORDER BY {}",
        if wh.is_empty() { String::new() } else { format!("WHERE {}", wh.join(" AND ")) },
        order
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), |r| {
        Ok(PhotoLite {
            id: r.get(0)?,
            path: r.get(1)?,
            filename: r.get(2)?,
            folder: r.get(3)?,
            ext: r.get(4)?,
            rating: r.get(5)?,
            label: r.get(6)?,
            taken_at: r.get(7)?,
            width: r.get(8)?,
            height: r.get(9)?,
            lat: r.get(10)?,
            lon: r.get(11)?,
            thumb: r.get::<_, i64>(12)? != 0,
            mtime: r.get(13)?,
            root_id: r.get(14)?,
            orientation: r.get(15)?,
            stack_id: r.get(16)?,
            stack_top: r.get::<_, i64>(17)? != 0,
            stack_count: r.get(18)?,
        })
    })?;
    let mut out = vec![];
    for r in rows {
        out.push(r?);
    }
    if q.scope == "ids" {
        // Keep the order the caller asked for (e.g. AI ranking).
        if let Some(ids) = &q.ids {
            if q.sort.is_none() {
                let pos: std::collections::HashMap<i64, usize> =
                    ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
                out.sort_by_key(|p| pos.get(&p.id).copied().unwrap_or(usize::MAX));
            }
        }
    }
    Ok(out)
}

pub fn get_photo(conn: &Connection, id: i64) -> Result<Option<Photo>> {
    Ok(conn
        .query_row(&format!("SELECT {PHOTO_COLS} FROM photos WHERE id = ?1"), [id], photo_from_row)
        .optional()?)
}

pub fn get_photos(conn: &Connection, ids: &[i64]) -> Result<Vec<Photo>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let list: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    let mut stmt =
        conn.prepare(&format!("SELECT {PHOTO_COLS} FROM photos WHERE id IN ({})", list.join(",")))?;
    let rows = stmt.query_map([], photo_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn photo_by_path(conn: &Connection, path: &str) -> Result<Option<(i64, i64, i64)>> {
    Ok(conn
        .query_row("SELECT id, size, mtime FROM photos WHERE path = ?1", [path], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?)
}

pub fn list_roots(conn: &Connection) -> Result<Vec<Root>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.path, r.name, (SELECT COUNT(*) FROM photos p WHERE p.root_id = r.id) FROM roots r ORDER BY r.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        let path: String = r.get(1)?;
        Ok(Root {
            id: r.get(0)?,
            online: Path::new(&path).is_dir(),
            path,
            name: r.get(2)?,
            count: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Find the library root that contains `path`, if any.
pub fn root_for_path(conn: &Connection, path: &Path) -> Result<Option<(i64, PathBuf)>> {
    let roots = list_roots(conn)?;
    let mut best: Option<(i64, PathBuf)> = None;
    for r in roots {
        let rp = PathBuf::from(&r.path);
        if path.starts_with(&rp) && best.as_ref().map_or(true, |(_, b)| rp.as_os_str().len() > b.as_os_str().len()) {
            best = Some((r.id, rp));
        }
    }
    Ok(best)
}

pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Replace a folder prefix in every photo path below it (used after folder moves/renames).
pub fn rewrite_prefix(conn: &Connection, old: &str, new: &str, new_root: Option<i64>) -> Result<usize> {
    let mut args = vec![];
    let clause = folder_clause(old, true, &mut args);
    let mut stmt = conn.prepare(&format!("SELECT id, path, folder FROM photos WHERE {clause}"))?;
    let rows: Vec<(i64, String, String)> = stmt
        .query_map(params_from_iter(args.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let tx = conn.unchecked_transaction()?;
    for (id, path, folder) in &rows {
        let np = format!("{}{}", new, &path[old.len()..]);
        let nf = format!("{}{}", new, &folder[old.len()..]);
        match new_root {
            Some(r) => tx.execute(
                "UPDATE photos SET path = ?1, folder = ?2, root_id = ?3 WHERE id = ?4",
                params![np, nf, r, id],
            )?,
            None => tx.execute("UPDATE photos SET path = ?1, folder = ?2 WHERE id = ?3", params![np, nf, id])?,
        };
    }
    tx.commit()?;
    Ok(rows.len())
}
