//! Smart collections: saved searches such as "5 stars from 2024" or "Canon RAW files
//! without keywords". A collection stores rules, not photos, so it stays up to date as
//! photos are added, rated, moved or removed.
//!
//! Each rule is `{ field, op, value, value2 }`. To add a new kind of rule (for example a
//! pick/reject flag), add one arm to `rule_sql` here and one entry to `FIELDS` in
//! `src/lib/collections.ts`. See `docs/smart-collections.md`.

use crate::metadata::{RAW_EXTS, VIDEO_EXTS};
use anyhow::{bail, Result};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Rule {
    pub field: String,
    pub op: String,
    #[serde(deserialize_with = "text_or_number")]
    pub value: String,
    /// Second value, for "between".
    #[serde(deserialize_with = "text_or_number")]
    pub value2: String,
}

/// Values arrive as text, or as numbers from number boxes; keep them as text.
fn text_or_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct RuleSet {
    /// true: a photo matches if any rule matches. false: every rule must match.
    pub any: bool,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub rules: RuleSet,
    pub count: i64,
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS collections (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            rules TEXT NOT NULL,
            position INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

/// The date a photo was taken, or the file's date when it has none (as the grid sorts).
const DATE: &str = "coalesce(taken_at, datetime(mtime/1000, 'unixepoch'))";
/// Width and height as the photo is shown (orientations 5 to 8 are turned a quarter).
const SHOWN_W: &str = "(CASE WHEN orientation BETWEEN 5 AND 8 THEN height ELSE width END)";
const SHOWN_H: &str = "(CASE WHEN orientation BETWEEN 5 AND 8 THEN width ELSE height END)";

fn ext_list(exts: &[&str]) -> String {
    exts.iter().map(|e| format!("'{e}'")).collect::<Vec<_>>().join(",")
}

fn type_sql(kind: &str) -> Result<String> {
    let raw = ext_list(RAW_EXTS);
    let video = ext_list(VIDEO_EXTS);
    Ok(match kind {
        "raw" => format!("lower(ext) IN ({raw})"),
        "jpeg" => "lower(ext) IN ('jpg','jpeg','jpe')".into(),
        "heic" => "lower(ext) IN ('heic','heif','avif')".into(),
        "png" => "lower(ext) = 'png'".into(),
        "tiff" => "lower(ext) IN ('tif','tiff')".into(),
        "video" => format!("lower(ext) IN ({video})"),
        "photo" => format!("lower(ext) NOT IN ({video})"),
        other => bail!("unknown file type \"{other}\""),
    })
}

fn num(s: &str) -> Result<f64> {
    let n = s.trim().parse::<f64>().map_err(|_| anyhow::anyhow!("\"{s}\" is not a number"))?;
    if !n.is_finite() {
        bail!("\"{s}\" is not a number");
    }
    Ok(n)
}

/// A number of days for "in the last N days", 0 to 100 years.
fn days(s: &str) -> Result<i64> {
    let n = num(s)?;
    if !(0.0..=36500.0).contains(&n) {
        bail!("\"{s}\" days is out of range (0 to 36500)");
    }
    Ok(n as i64)
}

/// A `YYYY-MM-DD` date, checked so bad input can't match by accident.
fn day(s: &str) -> Result<String> {
    let d = s.trim();
    if jiff::civil::Date::strptime("%Y-%m-%d", d).is_err() {
        bail!("\"{s}\" is not a date (use YYYY-MM-DD)");
    }
    Ok(d.to_string())
}

fn like(s: &str) -> Value {
    crate::db::like_contains(s.trim()).into()
}

/// The `|keyword|` pattern for an exact keyword.
fn keyword_like(s: &str) -> Value {
    format!("%|{}|%", crate::db::like_escape(&s.to_lowercase())).into()
}

/// The SQL condition for one rule, adding its values to `args`. `None` means the rule
/// is still incomplete (for example no value typed yet) and is left out.
pub fn rule_sql(r: &Rule, args: &mut Vec<Value>) -> Result<Option<String>> {
    let v = r.value.trim();
    let needs_value = !matches!(r.op.as_str(), "any" | "none" | "yes" | "no" | "landscape" | "portrait" | "square");
    if needs_value && v.is_empty() {
        return Ok(None);
    }
    let arg = |args: &mut Vec<Value>, x: Value| {
        args.push(x);
        format!("?{}", args.len())
    };
    let sql = match (r.field.as_str(), r.op.as_str()) {
        // Stars, 0 to 5.
        ("rating", "gte") => format!("rating >= {}", arg(args, (num(v)? as i64).into())),
        ("rating", "lte") => format!("rating <= {}", arg(args, (num(v)? as i64).into())),
        ("rating", "eq") => format!("rating = {}", arg(args, (num(v)? as i64).into())),

        // Colour label.
        ("label", "is") => format!("lower(ifnull(label,'')) = {}", arg(args, v.to_lowercase().into())),
        ("label", "is_not") => format!("lower(ifnull(label,'')) <> {}", arg(args, v.to_lowercase().into())),
        ("label", "any") => "ifnull(label,'') <> ''".into(),
        ("label", "none") => "ifnull(label,'') = ''".into(),

        // Keywords are stored as |a|b|c| so a whole keyword can be matched.
        ("keyword", "has") => format!("lower(ifnull(keywords,'')) LIKE {} ESCAPE '\\'", arg(args, keyword_like(v))),
        ("keyword", "has_not") => {
            format!("lower(ifnull(keywords,'')) NOT LIKE {} ESCAPE '\\'", arg(args, keyword_like(v)))
        }
        ("keyword", "any") => "ifnull(keywords,'') <> ''".into(),
        ("keyword", "none") => "ifnull(keywords,'') = ''".into(),

        // Any text Frameloft knows about the photo.
        ("text", "contains") | ("text", "not_contains") => {
            let a = arg(args, like(v));
            let hit = format!(
                "(lower(filename) LIKE {a} ESCAPE '\\' OR lower(ifnull(title,'')) LIKE {a} ESCAPE '\\' OR lower(ifnull(caption,'')) LIKE {a} ESCAPE '\\' OR lower(ifnull(keywords,'')) LIKE {a} ESCAPE '\\' OR lower(ifnull(place,'')) LIKE {a} ESCAPE '\\')"
            );
            if r.op == "contains" {
                hit
            } else {
                format!("NOT {hit}")
            }
        }

        ("filename", "contains") => format!("lower(filename) LIKE {} ESCAPE '\\'", arg(args, like(v))),
        ("filename", "not_contains") => format!("lower(filename) NOT LIKE {} ESCAPE '\\'", arg(args, like(v))),
        ("filename", "starts") => format!(
            "lower(filename) LIKE {} ESCAPE '\\'",
            arg(args, format!("{}%", crate::db::like_escape(&v.to_lowercase())).into())
        ),

        // A folder and everything below it.
        ("folder", "in") | ("folder", "not_in") => {
            let clause = crate::db::folder_clause(v, true, args);
            if r.op == "in" {
                clause
            } else {
                format!("NOT {clause}")
            }
        }

        // When it was taken.
        ("taken", "year") => format!("substr({DATE}, 1, 4) = {}", arg(args, format!("{:04}", num(v)? as i64).into())),
        ("taken", "month") => format!("CAST(substr({DATE}, 6, 2) AS INTEGER) = {}", arg(args, (num(v)? as i64).into())),
        ("taken", "after") => format!("substr({DATE}, 1, 10) >= {}", arg(args, day(v)?.into())),
        ("taken", "before") => format!("substr({DATE}, 1, 10) <= {}", arg(args, day(v)?.into())),
        ("taken", "between") => {
            if r.value2.trim().is_empty() {
                return Ok(None);
            }
            let (a, b) = (day(v)?, day(&r.value2)?);
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            format!("substr({DATE}, 1, 10) BETWEEN {} AND {}", arg(args, a.into()), arg(args, b.into()))
        }
        ("taken", "last_days") => {
            format!("{DATE} >= datetime('now', {})", arg(args, format!("-{} days", days(v)?).into()))
        }
        ("taken", "none") => "taken_at IS NULL".into(),

        // When Frameloft first saw it.
        ("added", "last_days") => {
            let cutoff = crate::db::now_ms().saturating_sub(days(v)? * 24 * 3600 * 1000);
            format!("indexed_at >= {}", arg(args, cutoff.into()))
        }

        // Camera: make and model together, e.g. "Canon EOS R5".
        ("camera", "contains") => {
            format!("lower(trim(ifnull(make,'') || ' ' || ifnull(model,''))) LIKE {} ESCAPE '\\'", arg(args, like(v)))
        }
        ("camera", "not_contains") => format!(
            "lower(trim(ifnull(make,'') || ' ' || ifnull(model,''))) NOT LIKE {} ESCAPE '\\'",
            arg(args, like(v))
        ),
        ("camera", "none") => "ifnull(make,'') = '' AND ifnull(model,'') = ''".into(),
        ("lens", "contains") => format!("lower(ifnull(lens,'')) LIKE {} ESCAPE '\\'", arg(args, like(v))),
        ("lens", "none") => "ifnull(lens,'') = ''".into(),

        ("type", "is") => type_sql(v)?,
        ("type", "is_not") => format!("NOT ({})", type_sql(v)?),

        ("location", "yes") => "lat IS NOT NULL AND lon IS NOT NULL".into(),
        ("location", "no") => "(lat IS NULL OR lon IS NULL)".into(),
        ("place", "contains") => format!("lower(ifnull(place,'')) LIKE {} ESCAPE '\\'", arg(args, like(v))),

        ("shape", "landscape") => format!("{SHOWN_W} > {SHOWN_H}"),
        ("shape", "portrait") => format!("{SHOWN_W} < {SHOWN_H}"),
        ("shape", "square") => format!("{SHOWN_W} = {SHOWN_H}"),

        ("iso", "gte") => format!("iso >= {}", arg(args, num(v)?.into())),
        ("iso", "lte") => format!("iso <= {}", arg(args, num(v)?.into())),
        ("focal", "gte") => format!("focal >= {}", arg(args, num(v)?.into())),
        ("focal", "lte") => format!("focal <= {}", arg(args, num(v)?.into())),
        ("fnumber", "gte") => format!("fnumber >= {}", arg(args, num(v)?.into())),
        ("fnumber", "lte") => format!("fnumber <= {}", arg(args, num(v)?.into())),

        (f, o) => bail!("unknown rule \"{f} {o}\""),
    };
    Ok(Some(sql))
}

/// The condition for a whole rule set, or `None` when it has no complete rules (which
/// matches every photo).
pub fn where_sql(set: &RuleSet, args: &mut Vec<Value>) -> Result<Option<String>> {
    let mut parts = vec![];
    for r in &set.rules {
        if let Some(s) = rule_sql(r, args)? {
            parts.push(format!("({s})"));
        }
    }
    if parts.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!("({})", parts.join(if set.any { " OR " } else { " AND " }))))
}

pub fn count(conn: &Connection, set: &RuleSet) -> Result<i64> {
    let mut args = vec![];
    let sql = match where_sql(set, &mut args)? {
        Some(w) => format!("SELECT COUNT(*) FROM photos WHERE {w}"),
        None => "SELECT COUNT(*) FROM photos".into(),
    };
    Ok(conn.query_row(&sql, params_from_iter(args.iter()), |r| r.get(0))?)
}

pub fn get_rules(conn: &Connection, id: i64) -> Result<Option<RuleSet>> {
    use rusqlite::OptionalExtension;
    let json: Option<String> =
        conn.query_row("SELECT rules FROM collections WHERE id = ?1", [id], |r| r.get(0)).optional()?;
    Ok(json.map(|j| serde_json::from_str(&j).unwrap_or_default()))
}

pub fn list(conn: &Connection) -> Result<Vec<Collection>> {
    let mut st = conn.prepare("SELECT id, name, rules FROM collections ORDER BY position, name COLLATE NOCASE")?;
    let rows: Vec<(i64, String, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut out = vec![];
    for (id, name, json) in rows {
        let rules: RuleSet = serde_json::from_str(&json).unwrap_or_default();
        // A rule Frameloft can't read (say, from a newer version) shows as empty, not as an error.
        let count = count(conn, &rules).unwrap_or(0);
        out.push(Collection { id, name, rules, count });
    }
    Ok(out)
}

/// Create (no `id`) or update a collection. Returns its id.
pub fn save(conn: &Connection, id: Option<i64>, name: &str, rules: &RuleSet) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        bail!("Give the collection a name.");
    }
    // Check the rules before saving them.
    where_sql(rules, &mut vec![])?;
    let json = serde_json::to_string(rules)?;
    match id {
        Some(id) => {
            let n =
                conn.execute("UPDATE collections SET name = ?1, rules = ?2 WHERE id = ?3", params![name, json, id])?;
            if n == 0 {
                bail!("That collection no longer exists.");
            }
            Ok(id)
        }
        None => {
            let pos: i64 = conn.query_row("SELECT ifnull(MAX(position), 0) + 1 FROM collections", [], |r| r.get(0))?;
            conn.execute(
                "INSERT INTO collections (name, rules, position, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![name, json, pos, crate::db::now_ms()],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM collections WHERE id = ?1", [id])?;
    Ok(())
}

pub mod cmd {
    use super::*;
    use crate::state::AppState;
    use std::sync::Arc;
    use tauri::State;

    type S<'a> = State<'a, Arc<AppState>>;
    fn e<E: std::fmt::Display>(x: E) -> String {
        x.to_string()
    }

    #[tauri::command]
    pub fn list_collections(state: S) -> Result<Vec<Collection>, String> {
        list(&state.conn.lock()).map_err(e)
    }

    #[tauri::command]
    pub fn save_collection(state: S, id: Option<i64>, name: String, rules: RuleSet) -> Result<i64, String> {
        save(&state.conn.lock(), id, &name, &rules).map_err(e)
    }

    #[tauri::command]
    pub fn delete_collection(state: S, id: i64) -> Result<(), String> {
        delete(&state.conn.lock(), id).map_err(e)
    }

    /// How many photos the rules match, for the editor's live count.
    #[tauri::command]
    pub fn count_rules(state: S, rules: RuleSet) -> Result<i64, String> {
        count(&state.conn.lock(), &rules).map_err(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::MAIN_SEPARATOR as SEP;

    fn rule(field: &str, op: &str, value: &str) -> Rule {
        Rule { field: field.into(), op: op.into(), value: value.into(), value2: String::new() }
    }

    fn all(rules: Vec<Rule>) -> RuleSet {
        RuleSet { any: false, rules }
    }

    /// A small catalogue with photos that differ in one way each.
    fn library() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        conn.execute("INSERT INTO roots (id, path, name, added_at) VALUES (1, '/lib', 'lib', 0)", []).unwrap();
        let add = |name: &str, folder: &str, ext: &str, sql: &str| {
            conn.execute(
                "INSERT INTO photos (root_id, path, folder, filename, ext, mtime, indexed_at) VALUES (1, ?1, ?2, ?3, ?4, 1700000000000, 0)",
                params![format!("{folder}{SEP}{name}"), folder, name, ext],
            )
            .unwrap();
            if !sql.is_empty() {
                conn.execute(&format!("UPDATE photos SET {sql} WHERE id = ?1"), [conn.last_insert_rowid()]).unwrap();
            }
        };
        let lib = format!("{SEP}lib");
        let trips = format!("{lib}{SEP}Trips");
        let rome = format!("{trips}{SEP}Rome");
        add("a.jpg", &rome, "jpg", "rating = 5, taken_at = '2024-06-01T10:00:00', make = 'Canon', model = 'EOS R5', keywords = '|Italy|Food|', width = 6000, height = 4000, orientation = 1");
        add("b.cr3", &rome, "cr3", "rating = 5, taken_at = '2023-12-24T10:00:00', make = 'Canon', model = 'EOS R5', label = 'Red', width = 6000, height = 4000, orientation = 6");
        add("c.jpg", &trips, "jpg", "rating = 3, taken_at = '2024-01-15T08:00:00', make = 'FUJIFILM', model = 'X-T5', lat = 51.5, lon = -0.1, place = 'London'");
        add("d.mp4", &lib, "mp4", "rating = 0");
        add("trips-notes.png", &format!("{lib}{SEP}Tripsy"), "png", "rating = 1, keywords = '|Italy food|'");
        conn
    }

    #[test]
    fn bad_numbers_are_errors_not_crashes() {
        let mut args = vec![];
        for v in ["NaN", "inf", "Infinity", "-inf"] {
            assert!(rule_sql(&rule("rating", "gte", v), &mut args).is_err(), "{v}");
        }
        // Used to overflow (and abort the app) when multiplied out to milliseconds.
        assert!(rule_sql(&rule("added", "last_days", "1000000000000"), &mut args).is_err());
        assert!(rule_sql(&rule("taken", "last_days", "-5"), &mut args).is_err());
        assert!(rule_sql(&rule("added", "last_days", "30"), &mut args).unwrap().is_some());
    }

    #[test]
    fn wildcards_in_text_are_literal() {
        let conn = library();
        conn.execute("UPDATE photos SET filename = '100%.jpg' WHERE filename = 'a.jpg'", []).unwrap();
        conn.execute("UPDATE photos SET filename = '1000.jpg' WHERE filename = 'c.jpg'", []).unwrap();
        assert_eq!(names(&conn, &all(vec![rule("filename", "contains", "100%")])), vec!["100%.jpg"]);
        assert_eq!(names(&conn, &all(vec![rule("filename", "contains", "1_0")])), Vec::<String>::new());
        assert_eq!(names(&conn, &all(vec![rule("filename", "starts", "100%")])), vec!["100%.jpg"]);
    }

    fn names(conn: &Connection, set: &RuleSet) -> Vec<String> {
        let mut args = vec![];
        let w = where_sql(set, &mut args).unwrap().unwrap_or_else(|| "1".into());
        let mut st = conn.prepare(&format!("SELECT filename FROM photos WHERE {w} ORDER BY filename")).unwrap();
        st.query_map(params_from_iter(args.iter()), |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    }

    #[test]
    fn five_stars_from_2024() {
        let c = library();
        let set = all(vec![rule("rating", "gte", "5"), rule("taken", "year", "2024")]);
        assert_eq!(names(&c, &set), ["a.jpg"]);
        assert_eq!(count(&c, &set).unwrap(), 1);
    }

    #[test]
    fn match_any() {
        let c = library();
        let set = RuleSet { any: true, rules: vec![rule("label", "is", "red"), rule("location", "yes", "")] };
        assert_eq!(names(&c, &set), ["b.cr3", "c.jpg"]);
    }

    #[test]
    fn empty_and_incomplete_rules_match_everything() {
        let c = library();
        assert_eq!(names(&c, &all(vec![])).len(), 5);
        assert_eq!(names(&c, &all(vec![rule("camera", "contains", "  ")])).len(), 5);
        let half =
            Rule { field: "taken".into(), op: "between".into(), value: "2024-01-01".into(), value2: String::new() };
        assert_eq!(names(&c, &all(vec![half])).len(), 5);
    }

    #[test]
    fn whole_keywords_only() {
        let c = library();
        assert_eq!(names(&c, &all(vec![rule("keyword", "has", "italy")])), ["a.jpg"]);
        assert_eq!(names(&c, &all(vec![rule("keyword", "none", "")])), ["b.cr3", "c.jpg", "d.mp4"]);
        assert_eq!(names(&c, &all(vec![rule("keyword", "has_not", "Food")])).len(), 4);
    }

    #[test]
    fn folders_include_subfolders_but_not_lookalike_names() {
        let c = library();
        let trips = format!("{SEP}lib{SEP}Trips");
        assert_eq!(names(&c, &all(vec![rule("folder", "in", &trips)])), ["a.jpg", "b.cr3", "c.jpg"]);
        assert_eq!(names(&c, &all(vec![rule("folder", "not_in", &trips)])), ["d.mp4", "trips-notes.png"]);
        // Folder rules number their own arguments; mixed with others they must still line up.
        let set = all(vec![rule("rating", "eq", "5"), rule("folder", "in", &trips), rule("taken", "year", "2023")]);
        assert_eq!(names(&c, &set), ["b.cr3"]);
    }

    #[test]
    fn dates() {
        let c = library();
        let between = Rule {
            field: "taken".into(),
            op: "between".into(),
            value: "2024-06-30".into(),
            value2: "2024-01-01".into(),
        };
        assert_eq!(names(&c, &all(vec![between])), ["a.jpg", "c.jpg"]);
        assert_eq!(names(&c, &all(vec![rule("taken", "month", "12")])), ["b.cr3"]);
        assert_eq!(names(&c, &all(vec![rule("taken", "none", "")])), ["d.mp4", "trips-notes.png"]);
        // Photos without a date taken use the file's date (2023-11-14 here).
        assert_eq!(names(&c, &all(vec![rule("taken", "year", "2023")])), ["b.cr3", "d.mp4", "trips-notes.png"]);
        assert!(where_sql(&all(vec![rule("taken", "after", "June 2024")]), &mut vec![]).is_err());
    }

    #[test]
    fn camera_type_and_shape() {
        let c = library();
        assert_eq!(names(&c, &all(vec![rule("camera", "contains", "canon r5")])), Vec::<String>::new());
        assert_eq!(names(&c, &all(vec![rule("camera", "contains", "eos r5")])), ["a.jpg", "b.cr3"]);
        assert_eq!(names(&c, &all(vec![rule("type", "is", "raw")])), ["b.cr3"]);
        assert_eq!(names(&c, &all(vec![rule("type", "is", "video")])), ["d.mp4"]);
        assert_eq!(names(&c, &all(vec![rule("type", "is", "photo")])).len(), 4);
        // b.cr3 is stored landscape but turned a quarter (orientation 6), so it shows portrait.
        assert_eq!(names(&c, &all(vec![rule("shape", "portrait", "")])), ["b.cr3"]);
        assert_eq!(names(&c, &all(vec![rule("shape", "landscape", "")])), ["a.jpg"]);
    }

    #[test]
    fn text_is_parameterised() {
        let c = library();
        // Quotes in values are data, never SQL.
        assert_eq!(names(&c, &all(vec![rule("text", "contains", "x' OR '1'='1")])), Vec::<String>::new());
        assert_eq!(names(&c, &all(vec![rule("text", "contains", "london")])), ["c.jpg"]);
    }

    #[test]
    fn numbers_and_text_values_both_work() {
        let set: RuleSet = serde_json::from_str(r#"{"any":false,"rules":[{"field":"taken","op":"year","value":2024,"value2":null},{"field":"rating","op":"gte","value":"5"}]}"#).unwrap();
        assert_eq!(set.rules[0].value, "2024");
        assert_eq!(set.rules[1].value2, "");
        assert_eq!(names(&library(), &set), ["a.jpg"]);
    }

    #[test]
    fn unknown_rules_are_refused() {
        assert!(where_sql(&all(vec![rule("colour", "is", "x")]), &mut vec![]).is_err());
        assert!(where_sql(&all(vec![rule("type", "is", "gif; DROP TABLE photos")]), &mut vec![]).is_err());
    }

    #[test]
    fn save_list_delete() {
        let c = library();
        let set = all(vec![rule("rating", "gte", "5")]);
        assert!(save(&c, None, "  ", &set).is_err());
        let id = save(&c, None, "Best", &set).unwrap();
        let l = list(&c).unwrap();
        assert_eq!((l[0].id, l[0].name.as_str(), l[0].count), (id, "Best", 2));
        save(&c, Some(id), "Best of 2024", &all(vec![rule("rating", "gte", "5"), rule("taken", "year", "2024")]))
            .unwrap();
        assert_eq!(list(&c).unwrap()[0].count, 1);
        assert_eq!(get_rules(&c, id).unwrap().unwrap().rules.len(), 2);
        delete(&c, id).unwrap();
        assert!(list(&c).unwrap().is_empty());
        assert!(save(&c, Some(id), "Gone", &set).is_err());
    }
}
