//! Commands called from the user interface.

use crate::db::{self, Photo, PhotoLite, Query, Root};
use crate::fileops::{self, OpReport};
use crate::indexer::{self, Job};
use crate::metadata::{self, Edit};
use crate::openwith::{self, AppInfo};
use crate::state::AppState;
use crate::{ai, watcher};
use rusqlite::params;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::sync::Arc;
use tauri::State;

type S<'a> = State<'a, Arc<AppState>>;
type R<T> = Result<T, String>;

fn e<E: std::fmt::Display>(x: E) -> String {
    x.to_string()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> R<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(e)?.map_err(e)
}

// ------------------------------------------------------------ library roots

#[tauri::command]
pub fn get_roots(state: S) -> R<Vec<Root>> {
    db::list_roots(&state.conn.lock()).map_err(e)
}

#[tauri::command]
pub fn add_root(state: S, path: String) -> R<i64> {
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Err("That folder is not available.".into());
    }
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
    let id = {
        let conn = state.conn.lock();
        // A folder inside an existing library folder is already covered.
        let mut children = vec![];
        for r in db::list_roots(&conn).map_err(e)? {
            let rp = PathBuf::from(&r.path);
            if p.starts_with(&rp) {
                return Err(format!("This folder is already in your library (inside \"{}\").", r.name));
            }
            if rp.starts_with(&p) {
                children.push(r.id);
            }
        }
        // Adding a parent of existing library folders absorbs them: their photos (with
        // thumbnails, ratings and everything else) move under the new folder.
        let tx = conn.unchecked_transaction().map_err(e)?;
        tx.execute(
            "INSERT INTO roots (path, name, added_at) VALUES (?1, ?2, ?3)",
            params![path, name, chrono::Utc::now().timestamp_millis()],
        )
        .map_err(e)?;
        let id = tx.last_insert_rowid();
        for c in &children {
            tx.execute("UPDATE photos SET root_id = ?1 WHERE root_id = ?2", params![id, c]).map_err(e)?;
            tx.execute("DELETE FROM roots WHERE id = ?1", [c]).map_err(e)?;
        }
        tx.commit().map_err(e)?;
        id
    };
    state.queue(Job::ScanRoot(id));
    watcher::restart(&state);
    Ok(id)
}

#[tauri::command]
pub fn remove_root(state: S, id: i64) -> R<()> {
    {
        let conn = state.conn.lock();
        let ids: Vec<i64> = {
            let mut st = conn.prepare("SELECT id FROM photos WHERE root_id = ?1").map_err(e)?;
            let rows = st.query_map([id], |r| r.get(0)).map_err(e)?;
            rows.filter_map(|r| r.ok()).collect()
        };
        indexer::delete_ids(&state, &conn, &ids).map_err(e)?;
        conn.execute("DELETE FROM roots WHERE id = ?1", [id]).map_err(e)?;
    }
    watcher::restart(&state);
    Ok(())
}

#[tauri::command]
pub fn rescan(state: S, root_id: Option<i64>, folder: Option<String>) -> R<()> {
    if let Some(f) = folder {
        state.queue(Job::ScanFolder(PathBuf::from(f)));
    } else if let Some(id) = root_id {
        state.queue(Job::ScanRoot(id));
    } else {
        for r in db::list_roots(&state.conn.lock()).map_err(e)? {
            state.queue(Job::ScanRoot(r.id));
        }
    }
    watcher::restart(&state);
    Ok(())
}

#[tauri::command]
pub fn refresh_photos(state: S, ids: Vec<i64>) -> R<()> {
    let photos = db::get_photos(&state.conn.lock(), &ids).map_err(e)?;
    state.queue(Job::Refresh(photos.into_iter().map(|p| PathBuf::from(p.path)).collect()));
    Ok(())
}

// ------------------------------------------------------------ folders

#[derive(Serialize)]
pub struct FolderNode {
    pub path: String,
    pub name: String,
    pub count: i64,
    pub has_children: bool,
    pub online: bool,
}

#[tauri::command]
pub fn folder_children(state: S, path: String) -> R<Vec<FolderNode>> {
    let base = path.trim_end_matches(['/', '\\']).to_string();
    let base = if base.is_empty() { path.clone() } else { base };
    let prefix = format!("{base}{MAIN_SEPARATOR}");
    // Photo counts per immediate child, from the catalogue.
    let mut counts: HashMap<String, i64> = HashMap::new();
    let mut deeper: HashMap<String, bool> = HashMap::new();
    {
        let conn = state.conn.lock();
        let mut args = vec![];
        let clause = db::folder_clause(&base, true, &mut args);
        let mut st = conn.prepare(&format!("SELECT folder, COUNT(*) FROM photos WHERE {clause} GROUP BY folder")).map_err(e)?;
        let rows = st
            .query_map(rusqlite::params_from_iter(args.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(e)?;
        for (folder, n) in rows.flatten() {
            if let Some(rest) = folder.strip_prefix(&prefix) {
                let mut parts = rest.splitn(2, MAIN_SEPARATOR);
                let child = parts.next().unwrap_or("").to_string();
                if child.is_empty() {
                    continue;
                }
                *counts.entry(child.clone()).or_default() += n;
                if parts.next().is_some() {
                    deeper.insert(child, true);
                }
            }
        }
    }
    let base_path = Path::new(&base);
    let mut out = vec![];
    if base_path.is_dir() {
        if let Ok(rd) = std::fs::read_dir(base_path) {
            for ent in rd.flatten() {
                let p = ent.path();
                if !ent.file_type().map(|t| t.is_dir()).unwrap_or(false) || indexer::is_hidden(&p) {
                    continue;
                }
                let name = ent.file_name().to_string_lossy().to_string();
                let has_children = std::fs::read_dir(&p)
                    .map(|rd| rd.flatten().any(|c| c.file_type().map(|t| t.is_dir()).unwrap_or(false) && !indexer::is_hidden(&c.path())))
                    .unwrap_or(false);
                out.push(FolderNode {
                    count: counts.get(&name).copied().unwrap_or(0),
                    path: p.to_string_lossy().to_string(),
                    name,
                    has_children,
                    online: true,
                });
            }
        }
    } else {
        for (name, count) in counts {
            out.push(FolderNode {
                path: format!("{prefix}{name}"),
                has_children: deeper.get(&name).copied().unwrap_or(false),
                name,
                count,
                online: false,
            });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

#[tauri::command]
pub async fn create_folder(state: S<'_>, parent: String, name: String) -> R<String> {
    let st = state.inner().clone();
    blocking(move || {
        let p = fileops::create_folder(Path::new(&parent), &name)?;
        st.queue(Job::ScanFolder(p.clone()));
        Ok(p.to_string_lossy().to_string())
    })
    .await
}

#[tauri::command]
pub async fn rename_folder(state: S<'_>, path: String, name: String) -> R<String> {
    let st = state.inner().clone();
    let r = blocking(move || Ok(fileops::rename_folder(&st, Path::new(&path), &name)?.to_string_lossy().to_string())).await;
    watcher::restart(&state);
    r
}

#[tauri::command]
pub async fn delete_folder(state: S<'_>, path: String, permanent: bool) -> R<()> {
    let st = state.inner().clone();
    blocking(move || fileops::delete_folder(&st, Path::new(&path), permanent)).await
}

#[tauri::command]
pub async fn move_folder(state: S<'_>, src: String, dest: String, copy: bool) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || fileops::move_folder(&st, Path::new(&src), Path::new(&dest), copy)).await
}

// ------------------------------------------------------------ photos

#[tauri::command]
pub fn query_photos(state: S, q: Query) -> R<Vec<PhotoLite>> {
    db::query_photos(&state.conn.lock(), &q).map_err(e)
}

#[tauri::command]
pub fn get_photo(state: S, id: i64) -> R<Option<Photo>> {
    db::get_photo(&state.conn.lock(), id).map_err(e)
}

#[tauri::command]
pub fn get_photos(state: S, ids: Vec<i64>) -> R<Vec<Photo>> {
    db::get_photos(&state.conn.lock(), &ids).map_err(e)
}

#[tauri::command]
pub async fn move_photos(state: S<'_>, ids: Vec<i64>, dest: String, copy: bool) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || fileops::move_photos(&st, &ids, Path::new(&dest), copy)).await
}

#[tauri::command]
pub async fn delete_photos(state: S<'_>, ids: Vec<i64>, permanent: bool) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || fileops::delete_photos(&st, &ids, permanent)).await
}

#[tauri::command]
pub async fn rename_photo(state: S<'_>, id: i64, name: String) -> R<()> {
    let st = state.inner().clone();
    blocking(move || fileops::rename_photo(&st, id, &name)).await
}

/// Write metadata to the files (or their XMP sidecars) and update the catalogue.
#[tauri::command]
pub async fn update_metadata(state: S<'_>, ids: Vec<i64>, edit: Edit) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || {
        let photos = db::get_photos(&st.conn.lock(), &ids)?;
        let raw_mode = db::get_setting(&st.conn.lock(), "raw_write_mode").unwrap_or_else(|| "sidecar".into());
        let mut rep = OpReport::default();
        for p in photos {
            let path = PathBuf::from(&p.path);
            // Work out the final keyword list.
            let final_kw: Option<Vec<String>> = if let Some(k) = &edit.keywords {
                Some(k.clone())
            } else if edit.add_keywords.is_some() || edit.remove_keywords.is_some() {
                let mut k = p.keywords.clone();
                for a in edit.add_keywords.iter().flatten() {
                    if !k.iter().any(|x| x.eq_ignore_ascii_case(a)) {
                        k.push(a.trim().to_string());
                    }
                }
                if let Some(rm) = &edit.remove_keywords {
                    k.retain(|x| !rm.iter().any(|r| r.eq_ignore_ascii_case(x)));
                }
                Some(k)
            } else {
                None
            };
            let res: anyhow::Result<()> = (|| {
                if !path.exists() {
                    anyhow::bail!("the file is not available (is its drive connected?)");
                }
                let use_sidecar = (metadata::is_raw(&p.ext) && raw_mode != "embed") || metadata::video_needs_sidecar(&p.ext);
                if use_sidecar {
                    let side = indexer::find_sidecar(&path).unwrap_or_else(|| path.with_extension("xmp"));
                    if !side.exists() {
                        st.et.create_sidecar(&path, &side)?;
                    }
                    st.et.write(&side, &metadata::write_args(&edit, final_kw.as_deref(), true))?;
                } else {
                    // MP4 and MOV hold XMP but not EXIF or IPTC.
                    let xmp_only = metadata::is_video(&p.ext);
                    st.et.write(&path, &metadata::write_args(&edit, final_kw.as_deref(), xmp_only))?;
                }
                Ok(())
            })();
            match res {
                Ok(()) => {
                    rep.done += 1;
                    let conn = st.conn.lock();
                    if let Some(r) = edit.rating { conn.execute("UPDATE photos SET rating=?1 WHERE id=?2", params![r, p.id])?; }
                    if let Some(v) = &edit.label { conn.execute("UPDATE photos SET label=?1 WHERE id=?2", params![nz(v), p.id])?; }
                    if let Some(v) = &edit.title { conn.execute("UPDATE photos SET title=?1 WHERE id=?2", params![nz(v), p.id])?; }
                    if let Some(v) = &edit.caption { conn.execute("UPDATE photos SET caption=?1 WHERE id=?2", params![nz(v), p.id])?; }
                    if let Some(v) = &edit.creator { conn.execute("UPDATE photos SET creator=?1 WHERE id=?2", params![nz(v), p.id])?; }
                    if let Some(v) = &edit.copyright { conn.execute("UPDATE photos SET copyright=?1 WHERE id=?2", params![nz(v), p.id])?; }
                    if let Some(k) = &final_kw { conn.execute("UPDATE photos SET keywords=?1 WHERE id=?2", params![db::encode_keywords(k), p.id])?; }
                    if let Some(g) = &edit.gps_change() {
                        let (la, lo) = g.map(|(a, b)| (Some(a), Some(b))).unwrap_or((None, None));
                        conn.execute("UPDATE photos SET lat=?1, lon=?2 WHERE id=?3", params![la, lo, p.id])?;
                    }
                    if let Ok(m) = std::fs::metadata(&path) {
                        let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
                        conn.execute("UPDATE photos SET size=?1, mtime=?2 WHERE id=?3", params![m.len() as i64, mtime, p.id])?;
                    }
                }
                Err(err) => rep.failed.push((p.filename.clone(), err.to_string())),
            }
        }
        Ok(rep)
    })
    .await
}

/// Turn or flip photos without re-encoding them: only the orientation tag changes.
/// RAW files keep it in their XMP sidecar (unless RAW files are set to be written directly).
#[tauri::command]
pub async fn rotate_photos(state: S<'_>, ids: Vec<i64>, op: String) -> R<OpReport> {
    let st = state.inner().clone();
    blocking(move || {
        let photos = db::get_photos(&st.conn.lock(), &ids)?;
        let raw_mode = db::get_setting(&st.conn.lock(), "raw_write_mode").unwrap_or_else(|| "sidecar".into());
        let mut rep = OpReport::default();
        for p in photos {
            let path = PathBuf::from(&p.path);
            let Some(new_o) = metadata::transform_orientation(p.orientation, &op) else {
                anyhow::bail!("unknown rotation {op}");
            };
            let res: anyhow::Result<()> = (|| {
                if !path.exists() {
                    anyhow::bail!("the file is not available (is its drive connected?)");
                }
                if metadata::is_video(&p.ext) {
                    anyhow::bail!("videos can't be rotated in Lumio");
                }
                if matches!(p.ext.as_str(), "gif" | "bmp") {
                    anyhow::bail!("{} files can't store a rotation", p.ext.to_uppercase());
                }
                if metadata::is_raw(&p.ext) && raw_mode != "embed" {
                    let side = indexer::find_sidecar(&path).unwrap_or_else(|| path.with_extension("xmp"));
                    if !side.exists() {
                        st.et.create_sidecar(&path, &side)?;
                    }
                    st.et.write(&side, &[format!("-XMP-tiff:Orientation#={new_o}")])?;
                } else {
                    st.et.write(&path, &[format!("-Orientation#={new_o}")])?;
                }
                Ok(())
            })();
            match res {
                Ok(()) => {
                    rep.done += 1;
                    let (size, mtime) = std::fs::metadata(&path)
                        .map(|m| {
                            let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
                            (m.len() as i64, t)
                        })
                        .unwrap_or((p.size, p.mtime));
                    let swap = metadata::is_quarter_turn(p.orientation) != metadata::is_quarter_turn(Some(new_o));
                    {
                        let conn = st.conn.lock();
                        conn.execute(
                            "UPDATE photos SET orientation = ?2, size = ?3, mtime = ?4,
                                width = CASE WHEN ?5 THEN height ELSE width END,
                                height = CASE WHEN ?5 THEN width ELSE height END WHERE id = ?1",
                            params![p.id, new_o, size, mtime, swap],
                        )?;
                    }
                    let dest = crate::thumbs::thumb_path(&st.thumbs_dir, p.id);
                    match crate::thumbs::make_thumb(&path, &p.ext, Some(new_o), &dest, &st.et, &st.tmp_dir) {
                        Ok(_) => {
                            st.conn.lock().execute("UPDATE photos SET thumb = 1 WHERE id = ?1", [p.id])?;
                        }
                        Err(err) => log::warn!("thumbnail after rotate failed for {}: {err:#}", path.display()),
                    }
                }
                Err(err) => {
                    log::warn!("rotate failed for {}: {err:#}", path.display());
                    rep.failed.push((p.filename.clone(), err.to_string()))
                }
            }
        }
        Ok(rep)
    })
    .await
}

/// Store a video thumbnail captured by the app's player (a JPEG as base64), for videos
/// the system could not make a thumbnail for. Also fills in the size and length if the
/// file's metadata did not have them.
#[tauri::command]
pub async fn save_video_thumb(state: S<'_>, id: i64, jpeg: String, width: Option<i64>, height: Option<i64>, duration: Option<f64>) -> R<()> {
    use base64::Engine;
    let st = state.inner().clone();
    blocking(move || {
        let Some(p) = db::get_photo(&st.conn.lock(), id)? else { anyhow::bail!("not in the catalogue") };
        if !metadata::is_video(&p.ext) {
            anyhow::bail!("not a video");
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(jpeg.trim())?;
        crate::thumbs::save_frame(&bytes, &crate::thumbs::thumb_path(&st.thumbs_dir, id))?;
        let duration = duration.filter(|d| d.is_finite() && *d > 0.0);
        st.conn.lock().execute(
            "UPDATE photos SET thumb = 1, thumb_tries = 0, width = coalesce(width, ?2), height = coalesce(height, ?3), duration = coalesce(duration, ?4) WHERE id = ?1",
            params![id, width.filter(|w| *w > 0), height.filter(|h| *h > 0), duration],
        )?;
        Ok(())
    })
    .await
}

// ------------------------------------------------------------ stacks (groups of shots)

/// Group photos into one stack. Stacks the photos already belong to are merged in.
/// `top` is the cover photo shown when the stack is closed.
#[tauri::command]
pub fn stack_photos(state: S, ids: Vec<i64>, top: Option<i64>) -> R<i64> {
    if ids.len() < 2 {
        return Err("Select at least two photos to group".into());
    }
    let conn = state.conn.lock();
    let tx = conn.unchecked_transaction().map_err(e)?;
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let sid: i64 = tx
        .query_row("SELECT coalesce(MAX(stack_id), 0) + 1 FROM photos", [], |r| r.get(0))
        .map_err(e)?;
    let top = top.filter(|t| ids.contains(t)).unwrap_or(ids[0]);
    tx.execute(
        &format!(
            "UPDATE photos SET stack_id = ?1, stack_top = (id = ?2) WHERE id IN ({list})
             OR stack_id IN (SELECT stack_id FROM photos WHERE id IN ({list}) AND stack_id IS NOT NULL)"
        ),
        params![sid, top],
    )
    .map_err(e)?;
    tx.commit().map_err(e)?;
    Ok(sid)
}

/// Take photos out of their stacks. With `whole`, every stack they belong to is dissolved.
#[tauri::command]
pub fn unstack_photos(state: S, ids: Vec<i64>, whole: bool) -> R<()> {
    let conn = state.conn.lock();
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let sql = if whole {
        format!("UPDATE photos SET stack_id = NULL, stack_top = 0 WHERE stack_id IN (SELECT stack_id FROM photos WHERE id IN ({list}) AND stack_id IS NOT NULL)")
    } else {
        format!("UPDATE photos SET stack_id = NULL, stack_top = 0 WHERE id IN ({list})")
    };
    conn.execute(&sql, []).map_err(e)?;
    db::tidy_stacks(&conn).map_err(e)
}

/// Make this photo the cover of its stack.
#[tauri::command]
pub fn set_stack_top(state: S, id: i64) -> R<()> {
    let conn = state.conn.lock();
    conn.execute(
        "UPDATE photos SET stack_top = (id = ?1) WHERE stack_id = (SELECT stack_id FROM photos WHERE id = ?1)",
        [id],
    )
    .map_err(e)?;
    Ok(())
}

fn nz(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

// ------------------------------------------------------------ open with

#[tauri::command]
pub fn list_open_with(state: S, id: i64) -> R<Vec<AppInfo>> {
    let p = db::get_photo(&state.conn.lock(), id).map_err(e)?.ok_or("photo not found")?;
    Ok(openwith::list(Path::new(&p.path)))
}

#[tauri::command]
pub fn open_with(state: S, ids: Vec<i64>, app: String) -> R<()> {
    let photos = db::get_photos(&state.conn.lock(), &ids).map_err(e)?;
    for p in photos {
        openwith::open_with(Path::new(&p.path), &app).map_err(e)?;
    }
    Ok(())
}

// ------------------------------------------------------------ settings

#[tauri::command]
pub fn get_settings(state: S) -> R<HashMap<String, String>> {
    let conn = state.conn.lock();
    let mut st = conn.prepare("SELECT key, value FROM settings WHERE key NOT LIKE 'key_%'").map_err(e)?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn set_setting(state: S, key: String, value: String) -> R<()> {
    if key.starts_with("key_") {
        return Err("use ai_set_key".into());
    }
    db::set_setting(&state.conn.lock(), &key, &value).map_err(e)
}

#[derive(Serialize)]
pub struct AppStatus {
    pub version: String,
    pub exiftool: bool,
    pub exiftool_path: String,
    pub data_dir: String,
    pub photo_count: i64,
}

#[tauri::command]
pub fn app_status(state: S) -> R<AppStatus> {
    let n: i64 = state.conn.lock().query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0)).map_err(e)?;
    Ok(AppStatus {
        version: env!("CARGO_PKG_VERSION").into(),
        exiftool: state.et.available(),
        exiftool_path: state.et.describe(),
        data_dir: state.db_path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        photo_count: n,
    })
}

// ------------------------------------------------------------ AI

#[tauri::command]
pub fn ai_settings(state: S) -> ai::AiSettings {
    ai::settings(&state)
}

#[tauri::command]
pub fn ai_set_key(state: S, which: String, key: String) -> R<()> {
    ai::set_key(&state, &which, &key).map_err(e)
}

#[tauri::command]
pub async fn ai_chat(state: S<'_>, text: String) -> R<ai::ChatReply> {
    let st = state.inner().clone();
    ai::chat(&st, text).await.map_err(e)
}

#[tauri::command]
pub fn ai_reset(state: S) {
    ai::reset(&state)
}

// ------------------------------------------------------------ geocoding (OpenStreetMap Nominatim)

#[derive(Serialize)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[tauri::command]
pub async fn geocode(query: String) -> R<Vec<Place>> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("Lumio/", env!("CARGO_PKG_VERSION"), " (open-source photo manager)"))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(e)?;
    let v: serde_json::Value = client
        .get("https://nominatim.openstreetmap.org/search")
        .query(&[("q", query.as_str()), ("format", "jsonv2"), ("limit", "6")])
        .send()
        .await
        .map_err(e)?
        .json()
        .await
        .map_err(e)?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(Place {
                name: p["display_name"].as_str()?.to_string(),
                lat: p["lat"].as_str()?.parse().ok()?,
                lon: p["lon"].as_str()?.parse().ok()?,
            })
        })
        .collect())
}

// ------------------------------------------------------------ system integration

#[tauri::command]
pub fn reveal(state: S, id: Option<i64>, path: Option<String>) -> R<()> {
    let p = match (id, path) {
        (Some(id), _) => db::get_photo(&state.conn.lock(), id).map_err(e)?.ok_or("photo not found")?.path,
        (None, Some(p)) => p,
        _ => return Err("nothing to show".into()),
    };
    if !Path::new(&p).exists() {
        return Err("This item is not available (is its drive connected?)".into());
    }
    tauri_plugin_opener::reveal_item_in_dir(&p).map_err(e)
}

#[tauri::command]
pub fn open_default(state: S, ids: Vec<i64>) -> R<()> {
    for p in db::get_photos(&state.conn.lock(), &ids).map_err(e)? {
        if !Path::new(&p.path).exists() {
            return Err(format!("{} is not available (is its drive connected?)", p.filename));
        }
        tauri_plugin_opener::open_path(&p.path, None::<&str>).map_err(e)?;
    }
    Ok(())
}
