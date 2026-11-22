//! AI photo finder. The model gets tools to search the catalogue (and optionally
//! look at thumbnails) and replies with a set of matching photos.
//!
//! Providers:
//! * Anthropic Claude (Messages API, raw HTTP since there is no official Rust SDK)
//! * Any OpenAI-compatible endpoint, e.g. a local Ollama or LM Studio server.

use crate::db;
use crate::state::AppState;
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use rusqlite::params_from_iter;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Emitter;

pub const ANTHROPIC_VERSION: &str = "2026-10-21";
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-5-5";

#[derive(Default)]
pub struct Conversation {
    /// Provider-native message history (Anthropic content blocks are kept verbatim,
    /// including thinking blocks, as the API requires).
    pub messages: Vec<Value>,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiSettings {
    pub provider: String,
    pub anthropic_model: String,
    pub effort: String,
    pub openai_base_url: String,
    pub openai_model: String,
    pub allow_images: bool,
    pub has_anthropic_key: bool,
    pub has_openai_key: bool,
}

pub fn settings(state: &AppState) -> AiSettings {
    let conn = state.conn.lock();
    let g = |k: &str, d: &str| db::get_setting(&conn, k).unwrap_or_else(|| d.to_string());
    AiSettings {
        provider: g("ai_provider", "anthropic"),
        anthropic_model: g("ai_anthropic_model", DEFAULT_ANTHROPIC_MODEL),
        effort: g("ai_effort", "low"),
        openai_base_url: g("ai_openai_base_url", "http://localhost:11434/v1"),
        openai_model: g("ai_openai_model", "llama3.1"),
        allow_images: g("ai_allow_images", "1") == "1",
        has_anthropic_key: crate::keys::get("anthropic").is_some(),
        has_openai_key: crate::keys::get("openai").is_some(),
    }
}

/// Saves (or, when empty, removes) the API key for "anthropic" or "openai".
pub fn set_key(which: &str, key: &str) -> Result<()> {
    if !matches!(which, "anthropic" | "openai") {
        bail!("unknown provider {which}");
    }
    crate::keys::set(which, key.trim())
}

const SYSTEM_PROMPT: &str = "You are the photo-finding assistant inside Lumio, a desktop photo manager. \
The user asks in plain language for photos in their own catalogue. Use the tools to find them:\n\
- `library_overview` tells you what exists (folders, keywords, cameras, dates, places).\n\
- `search_photos` filters the catalogue by text, keywords, dates, ratings, colour labels, camera, lens, folder and location. \
For places, convert the place name to latitude/longitude yourself and use `near` with a sensible radius, and also try the `text` filter because some photos have place names but no GPS.\n\
- `view_photos` shows you small thumbnails when the metadata alone cannot answer a visual question (e.g. 'sunsets', 'photos with dogs'). Look at a modest number of likely candidates rather than everything.\n\
- `show_results` puts the photos you found in front of the user. Always call it when you have found photos, best matches first.\n\
Keep replies short and friendly: one or two sentences saying what you found and any assumptions. \
If nothing matches, say so and suggest a broader search. Never invent photo ids.";

fn tools_spec() -> Vec<Value> {
    vec![
        json!({
            "name": "library_overview",
            "description": "Summary of the catalogue: photo count, date range, library folders, the most common keywords, cameras, lenses, places, ratings and colour labels.",
            "input_schema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "search_photos",
            "description": "Search the photo catalogue. All filters are optional and combined with AND. Returns matching photos with their metadata (id, file, folder, date, rating, label, title, caption, keywords, camera, lens, gps, place).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "Words that must all appear somewhere in filename, title, caption, keywords, place, folder, camera or lens (case-insensitive)."},
                    "any_text": {"type": "array", "items": {"type": "string"}, "description": "Match photos containing ANY of these words (synonyms)."},
                    "keywords": {"type": "array", "items": {"type": "string"}, "description": "Exact keywords that must all be present."},
                    "folder": {"type": "string", "description": "Only photos inside this folder path (including subfolders)."},
                    "date_from": {"type": "string", "description": "YYYY-MM-DD, inclusive."},
                    "date_to": {"type": "string", "description": "YYYY-MM-DD, inclusive."},
                    "months": {"type": "array", "items": {"type": "integer"}, "description": "Only these months (1-12), any year. Useful for seasons."},
                    "min_rating": {"type": "integer", "minimum": 0, "maximum": 5},
                    "max_rating": {"type": "integer", "minimum": 0, "maximum": 5},
                    "label": {"type": "string", "description": "Colour label: Red, Yellow, Green, Blue or Purple."},
                    "camera": {"type": "string", "description": "Part of the camera make or model."},
                    "lens": {"type": "string", "description": "Part of the lens name."},
                    "has_location": {"type": "boolean"},
                    "near": {"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}, "radius_km": {"type": "number"}}, "required": ["lat", "lon"]},
                    "sort": {"type": "string", "enum": ["newest", "oldest", "rating"]},
                    "limit": {"type": "integer", "description": "Max photos to return (default 60, max 400)."}
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "view_photos",
            "description": "Look at small thumbnails of up to 16 photos by id to judge their visual content.",
            "input_schema": {"type": "object", "properties": {"ids": {"type": "array", "items": {"type": "integer"}, "maxItems": 16}}, "required": ["ids"], "additionalProperties": false}
        }),
        json!({
            "name": "show_results",
            "description": "Display these photos to the user in the app, in this order (best first).",
            "input_schema": {"type": "object", "properties": {"ids": {"type": "array", "items": {"type": "integer"}}, "title": {"type": "string", "description": "Short title for the result set, e.g. 'Beach photos, 2023'."}}, "required": ["ids"], "additionalProperties": false}
        }),
    ]
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ChatReply {
    pub text: String,
    pub result_ids: Option<Vec<i64>>,
    pub result_title: Option<String>,
}

fn emit_status(state: &AppState, msg: &str) {
    let _ = state.app.emit("ai-status", msg.to_string());
}

pub fn reset(state: &AppState) {
    let mut c = state.chat.lock();
    c.messages.clear();
}

/// One question at a time: a second one while the first is still being answered would
/// race on the conversation history.
static BUSY: AtomicBool = AtomicBool::new(false);

struct Turn;
impl Turn {
    fn start() -> Result<Turn> {
        if BUSY.swap(true, Ordering::AcqRel) {
            bail!("Still working on your previous question. Please wait for the answer.");
        }
        Ok(Turn)
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

/// Most rounds of tool use per question.
const MAX_ROUNDS: usize = 12;
/// Messages kept from earlier questions (older ones are dropped whole).
const MAX_HISTORY: usize = 60;

pub async fn chat(state: Arc<AppState>, user_text: String) -> Result<ChatReply> {
    let _turn = Turn::start()?;
    let s = settings(&state);
    {
        let mut c = state.chat.lock();
        if c.provider != s.provider {
            c.messages.clear();
            c.provider = s.provider.clone();
        }
        // Thumbnails from earlier questions have done their job; they would be sent
        // again with every new question otherwise.
        let old = std::mem::take(&mut c.messages);
        c.messages = trim_history(strip_old_images(old), MAX_HISTORY);
    }
    let r = if s.provider == "openai" { chat_openai(&state, &s, user_text).await } else { chat_anthropic(&state, &s, user_text).await };
    emit_status(&state, "");
    r
}

/// Keeps at most `max` messages, cutting only at the start of an earlier question (a user
/// message with text, not tool results), so no tool call loses its answer.
fn trim_history(messages: Vec<Value>, max: usize) -> Vec<Value> {
    if messages.len() <= max {
        return messages;
    }
    let from = messages.len() - max;
    let start = (from..messages.len()).find(|&i| is_question(&messages[i]));
    match start {
        Some(i) => {
            // A system message (OpenAI-style) stays at the top.
            let mut out: Vec<Value> = messages.iter().take(1).filter(|m| m["role"] == "system").cloned().collect();
            out.extend(messages.into_iter().skip(i));
            out
        }
        None => vec![],
    }
}

fn is_question(m: &Value) -> bool {
    if m["role"] != "user" {
        return false;
    }
    match &m["content"] {
        Value::String(_) => true,
        Value::Array(a) => a.iter().all(|b| b["type"] == "text"),
        _ => false,
    }
}

/// Reads a reply as JSON, or explains what came back instead (a proxy's HTML error page,
/// a gateway timeout…).
async fn json_reply(resp: reqwest::Response, what: &str) -> Result<(reqwest::StatusCode, Value)> {
    let status = resp.status();
    let text = resp.text().await?;
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => Ok((status, v)),
        Err(_) => {
            let snippet: String = text.chars().filter(|c| !c.is_control()).take(120).collect();
            bail!("{what} sent an unexpected reply ({}): {}", status.as_u16(), crate::secure::redact_body(&snippet));
        }
    }
}

/// Where an OpenAI-compatible server may be: `https://` anywhere public, or plain
/// `http://` on this computer only (Ollama, LM Studio). Link-local and cloud metadata
/// addresses are refused, so photo data and keys can't be sent there.
pub fn check_base_url(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| anyhow!("\"{raw}\" is not a web address."))?;
    if !url.username().is_empty() || url.password().is_some() {
        bail!("Put the key in the API key field, not in the address.");
    }
    let host = url.host().ok_or_else(|| anyhow!("The AI server address needs a host name."))?;
    let loopback = match &host {
        url::Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
    };
    let link_local = match &host {
        url::Host::Ipv4(ip) => ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast(),
        url::Host::Ipv6(ip) => (ip.segments()[0] & 0xffc0) == 0xfe80 || ip.is_unspecified(),
        url::Host::Domain(d) => d.eq_ignore_ascii_case("metadata.google.internal") || d.eq_ignore_ascii_case("metadata"),
    };
    if link_local {
        bail!("That address can't be used for an AI server.");
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback => Ok(url),
        "http" => bail!("Use https:// for an AI server on another computer (plain http:// is only allowed for this computer, e.g. http://localhost:11434/v1)."),
        _ => bail!("The AI server address must start with https:// (or http://localhost)."),
    }
}

// ------------------------------------------------------------------ tools

/// Runs a tool off the async threads: searching the catalogue and reading thumbnails is
/// blocking database and disk work.
async fn tool(state: &Arc<AppState>, name: &str, input: &Value, reply: &mut ChatReply, allow_images: bool) -> (Vec<Value>, bool) {
    let (st, name, input) = (state.clone(), name.to_string(), input.clone());
    let res = tauri::async_runtime::spawn_blocking(move || {
        let mut r = ChatReply::default();
        let out = run_tool(&st, &name, &input, &mut r, allow_images);
        (out, r)
    })
    .await;
    match res {
        Ok((out, r)) => {
            if r.result_ids.is_some() {
                reply.result_ids = r.result_ids;
                reply.result_title = r.result_title;
            }
            out
        }
        Err(e) => (vec![json!({"type": "text", "text": format!("tool failed: {e}")})], true),
    }
}

fn run_tool(state: &AppState, name: &str, input: &Value, reply: &mut ChatReply, allow_images: bool) -> (Vec<Value>, bool) {
    // Returns Anthropic-style content blocks for the tool result, plus an error flag.
    let r: Result<Vec<Value>> = (|| match name {
        "library_overview" => {
            emit_status(state, "Looking at your library…");
            Ok(vec![json!({"type": "text", "text": overview(state)?.to_string()})])
        }
        "search_photos" => {
            emit_status(state, "Searching the catalogue…");
            Ok(vec![json!({"type": "text", "text": search(state, input)?.to_string()})])
        }
        "view_photos" => {
            if !allow_images {
                bail!("Viewing thumbnails is turned off in Settings. Rely on metadata.");
            }
            emit_status(state, "Looking at photos…");
            let ids: Vec<i64> = input["ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_i64()).take(16).collect()).unwrap_or_default();
            let mut blocks = vec![];
            for id in ids {
                let p = crate::thumbs::thumb_path(&state.thumbs_dir, id);
                if let Ok(bytes) = std::fs::read(&p) {
                    blocks.push(json!({"type": "text", "text": format!("Photo id {id}:")}));
                    blocks.push(json!({"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": base64::engine::general_purpose::STANDARD.encode(bytes)}}));
                } else {
                    blocks.push(json!({"type": "text", "text": format!("Photo id {id}: no thumbnail available")}));
                }
            }
            Ok(blocks)
        }
        "show_results" => {
            let ids: Vec<i64> = input["ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_i64()).collect()).unwrap_or_default();
            reply.result_ids = Some(ids.clone());
            reply.result_title = input["title"].as_str().map(String::from);
            Ok(vec![json!({"type": "text", "text": format!("Showing {} photos to the user.", ids.len())})])
        }
        other => bail!("unknown tool {other}"),
    })();
    match r {
        Ok(b) => (b, false),
        Err(e) => (vec![json!({"type": "text", "text": e.to_string()})], true),
    }
}

fn overview(state: &AppState) -> Result<Value> {
    let conn = state.conn.lock();
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))?;
    let (dmin, dmax): (Option<String>, Option<String>) =
        conn.query_row("SELECT min(taken_at), max(taken_at) FROM photos", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let geo: i64 = conn.query_row("SELECT COUNT(*) FROM photos WHERE lat IS NOT NULL", [], |r| r.get(0))?;
    let top = |sql: &str| -> Result<Vec<Value>> {
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map([], |r| Ok(json!([r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?])))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    };
    // Keywords need splitting.
    let mut kw: std::collections::HashMap<String, i64> = Default::default();
    {
        let mut st = conn.prepare("SELECT keywords FROM photos WHERE keywords IS NOT NULL")?;
        for k in st.query_map([], |r| r.get::<_, String>(0))?.flatten() {
            for w in db::decode_keywords(Some(&k)) {
                *kw.entry(w).or_default() += 1;
            }
        }
    }
    let mut kw: Vec<(String, i64)> = kw.into_iter().collect();
    kw.sort_by(|a, b| b.1.cmp(&a.1));
    kw.truncate(80);
    let roots: Vec<Value> = db::list_roots(&conn)?.into_iter().map(|r| json!({"path": r.path, "photos": r.count, "online": r.online})).collect();
    Ok(json!({
        "total_photos": total,
        "date_range": [dmin, dmax],
        "geotagged_photos": geo,
        "library_folders": roots,
        "top_keywords": kw,
        "folders": top("SELECT folder, COUNT(*) c FROM photos GROUP BY folder ORDER BY c DESC LIMIT 60")?,
        "cameras": top("SELECT trim(coalesce(make,'') || ' ' || coalesce(model,'')), COUNT(*) c FROM photos WHERE model IS NOT NULL GROUP BY 1 ORDER BY c DESC LIMIT 20")?,
        "lenses": top("SELECT lens, COUNT(*) c FROM photos WHERE lens IS NOT NULL GROUP BY lens ORDER BY c DESC LIMIT 20")?,
        "places": top("SELECT place, COUNT(*) c FROM photos WHERE place IS NOT NULL GROUP BY place ORDER BY c DESC LIMIT 40")?,
        "ratings": top("SELECT CAST(rating AS TEXT), COUNT(*) FROM photos GROUP BY rating")?,
        "labels": top("SELECT label, COUNT(*) FROM photos WHERE label IS NOT NULL GROUP BY label")?,
        "photos_per_year": top("SELECT substr(taken_at,1,4), COUNT(*) FROM photos WHERE taken_at IS NOT NULL GROUP BY 1 ORDER BY 1")?,
    }))
}

fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, lo1, la2, lo2) = (a.0.to_radians(), a.1.to_radians(), b.0.to_radians(), b.1.to_radians());
    let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    // Rounding can push h a hair above 1 for opposite points, and asin(>1) is NaN.
    6371.0 * 2.0 * h.clamp(0.0, 1.0).sqrt().asin()
}

fn search(state: &AppState, i: &Value) -> Result<Value> {
    let mut wh: Vec<String> = vec![];
    let mut args: Vec<rusqlite::types::Value> = vec![];
    let hay = "lower(filename || ' ' || ifnull(title,'') || ' ' || ifnull(caption,'') || ' ' || ifnull(keywords,'') || ' ' || ifnull(place,'') || ' ' || folder || ' ' || ifnull(make,'') || ' ' || ifnull(model,'') || ' ' || ifnull(lens,''))";
    if let Some(t) = i["text"].as_str() {
        for w in t.split_whitespace() {
            args.push(db::like_contains(w).into());
            wh.push(format!("{hay} LIKE ?{} ESCAPE '\\'", args.len()));
        }
    }
    if let Some(a) = i["any_text"].as_array() {
        let mut ors = vec![];
        for w in a.iter().filter_map(|v| v.as_str()) {
            args.push(db::like_contains(w).into());
            ors.push(format!("{hay} LIKE ?{} ESCAPE '\\'", args.len()));
        }
        if !ors.is_empty() {
            wh.push(format!("({})", ors.join(" OR ")));
        }
    }
    if let Some(a) = i["keywords"].as_array() {
        for k in a.iter().filter_map(|v| v.as_str()) {
            args.push(format!("%|{}|%", db::like_escape(&k.to_lowercase())).into());
            wh.push(format!("lower(ifnull(keywords,'')) LIKE ?{} ESCAPE '\\'", args.len()));
        }
    }
    if let Some(f) = i["folder"].as_str() {
        wh.push(db::folder_clause(f, true, &mut args));
    }
    if let Some(d) = i["date_from"].as_str() {
        args.push(d.to_string().into());
        wh.push(format!("taken_at >= ?{}", args.len()));
    }
    if let Some(d) = i["date_to"].as_str() {
        args.push(format!("{d}T99").into());
        wh.push(format!("taken_at <= ?{}", args.len()));
    }
    if let Some(m) = i["months"].as_array() {
        let ms: Vec<String> = m.iter().filter_map(|v| v.as_i64()).map(|v| format!("'{v:02}'")).collect();
        if !ms.is_empty() {
            wh.push(format!("substr(taken_at,6,2) IN ({})", ms.join(",")));
        }
    }
    if let Some(r) = i["min_rating"].as_i64() {
        args.push(r.into());
        wh.push(format!("rating >= ?{}", args.len()));
    }
    if let Some(r) = i["max_rating"].as_i64() {
        args.push(r.into());
        wh.push(format!("rating <= ?{}", args.len()));
    }
    if let Some(l) = i["label"].as_str() {
        args.push(l.to_lowercase().into());
        wh.push(format!("lower(label) = ?{}", args.len()));
    }
    if let Some(c) = i["camera"].as_str() {
        args.push(db::like_contains(c).into());
        wh.push(format!("lower(ifnull(make,'') || ' ' || ifnull(model,'')) LIKE ?{} ESCAPE '\\'", args.len()));
    }
    if let Some(c) = i["lens"].as_str() {
        args.push(db::like_contains(c).into());
        wh.push(format!("lower(ifnull(lens,'')) LIKE ?{} ESCAPE '\\'", args.len()));
    }
    match i["has_location"].as_bool() {
        Some(true) => wh.push("lat IS NOT NULL".into()),
        Some(false) => wh.push("lat IS NULL".into()),
        None => {}
    }
    let near = i.get("near").and_then(|n| Some((n["lat"].as_f64()?, n["lon"].as_f64()?, n["radius_km"].as_f64().unwrap_or(25.0))));
    if let Some((lat, lon, r)) = near {
        let dlat = r / 111.0;
        let dlon = r / (111.0 * lat.to_radians().cos().abs().max(0.01));
        for v in [lat - dlat, lat + dlat, lon - dlon, lon + dlon] {
            args.push(v.into());
        }
        let n = args.len();
        wh.push(format!("lat BETWEEN ?{} AND ?{} AND lon BETWEEN ?{} AND ?{}", n - 3, n - 2, n - 1, n));
    }
    let order = match i["sort"].as_str() {
        Some("oldest") => "taken_at ASC",
        Some("rating") => "rating DESC, taken_at DESC",
        _ => "taken_at DESC",
    };
    let limit = i["limit"].as_i64().unwrap_or(60).clamp(1, 400) as usize;
    let where_sql = if wh.is_empty() { String::new() } else { format!("WHERE {}", wh.join(" AND ")) };
    let conn = state.conn.lock();
    let (rows, total) = if let Some((lat, lon, r)) = near {
        // The box above is only a pre-filter; the exact distance needs every candidate.
        let sql = format!("SELECT {} FROM photos {where_sql} ORDER BY {order}", db::PHOTO_COLS);
        let mut st = conn.prepare(&sql)?;
        let mut rows: Vec<db::Photo> = st.query_map(params_from_iter(args.iter()), db::photo_from_row)?.filter_map(|r| r.ok()).collect();
        rows.retain(|p| match (p.lat, p.lon) {
            (Some(a), Some(b)) => haversine_km((lat, lon), (a, b)) <= r,
            _ => false,
        });
        let total = rows.len();
        (rows, total)
    } else {
        // Without a distance, SQLite counts and limits, so only `limit` rows are read.
        let total: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM photos {where_sql}"), params_from_iter(args.iter()), |r| r.get(0))?;
        let sql = format!("SELECT {} FROM photos {where_sql} ORDER BY {order} LIMIT {limit}", db::PHOTO_COLS);
        let mut st = conn.prepare(&sql)?;
        let rows: Vec<db::Photo> = st.query_map(params_from_iter(args.iter()), db::photo_from_row)?.filter_map(|r| r.ok()).collect();
        (rows, total as usize)
    };
    drop(conn);
    let photos: Vec<Value> = rows
        .into_iter()
        .take(limit)
        .map(|p| {
            let mut o = json!({"id": p.id, "file": p.filename, "folder": p.folder});
            let m = o.as_object_mut().unwrap();
            if let Some(v) = p.taken_at { m.insert("date".into(), v.into()); }
            if p.rating > 0 { m.insert("rating".into(), p.rating.into()); }
            if let Some(v) = p.label { m.insert("label".into(), v.into()); }
            if let Some(v) = p.title { m.insert("title".into(), v.into()); }
            if let Some(v) = p.caption { m.insert("caption".into(), v.chars().take(200).collect::<String>().into()); }
            if !p.keywords.is_empty() { m.insert("keywords".into(), p.keywords.into()); }
            if let Some(v) = p.model { m.insert("camera".into(), v.into()); }
            if let Some(v) = p.lens { m.insert("lens".into(), v.into()); }
            if let (Some(a), Some(b)) = (p.lat, p.lon) { m.insert("gps".into(), json!([(a * 1e4).round() / 1e4, (b * 1e4).round() / 1e4])); }
            if let Some(v) = p.place { m.insert("place".into(), v.into()); }
            o
        })
        .collect();
    Ok(json!({"total_matches": total, "returned": photos.len(), "photos": photos}))
}

// ------------------------------------------------------------------ Anthropic

fn supports_fallbacks(model: &str) -> bool {
    matches!(model, "claude-opus-5-5" | "claude-sonnet-5-5" | "claude-fable-5-2" | "claude-fable-5-1")
}

async fn chat_anthropic(state: &Arc<AppState>, s: &AiSettings, user_text: String) -> Result<ChatReply> {
    let key = crate::keys::get("anthropic")
        .ok_or_else(|| anyhow!("Add your Anthropic API key in Settings → AI to use Claude."))?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build()?;
    let mut messages = { state.chat.lock().messages.clone() };
    messages.push(json!({"role": "user", "content": [{"type": "text", "text": user_text}]}));
    let mut reply = ChatReply::default();
    let mut finished = false;
    for _round in 0..MAX_ROUNDS {
        emit_status(state, "Thinking…");
        let mut body = json!({
            "model": s.anthropic_model,
            "max_tokens": 16000,
            "system": [{"type": "text", "text": SYSTEM_PROMPT, "cache_control": {"type": "ephemeral"}}],
            "tools": tools_spec(),
            "messages": messages,
            "output_config": {"effort": s.effort},
            // Automatic prompt caching of the growing conversation.
            "cache_control": {"type": "ephemeral"},
        });
        let mut req = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json");
        if supports_fallbacks(&s.anthropic_model) {
            body["fallbacks"] = json!("default");
            req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
        }
        let resp = req.json(&body).send().await?;
        let (status, v) = json_reply(resp, "Claude").await?;
        if !status.is_success() {
            let msg = v["error"]["message"].as_str().unwrap_or("unknown error");
            bail!("Claude API error ({}): {msg}", status.as_u16());
        }
        let content = v["content"].clone();
        messages.push(json!({"role": "assistant", "content": content}));
        let stop = v["stop_reason"].as_str().unwrap_or("");
        let text: String = content
            .as_array()
            .map(|a| a.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        if stop == "refusal" {
            reply.text = "Sorry, I can't help with that request.".into();
            finished = true;
            break;
        }
        if stop != "tool_use" {
            reply.text = text;
            finished = true;
            break;
        }
        let mut results = vec![];
        let uses: Vec<Value> = content.as_array().into_iter().flatten().filter(|b| b["type"] == "tool_use").cloned().collect();
        for b in uses {
            let (blocks, is_error) = tool(state, b["name"].as_str().unwrap_or(""), &b["input"], &mut reply, s.allow_images).await;
            results.push(json!({"type": "tool_result", "tool_use_id": b["id"], "content": blocks, "is_error": is_error}));
        }
        messages.push(json!({"role": "user", "content": results}));
        if !text.is_empty() {
            reply.text = text;
        }
    }
    if !finished {
        // Out of rounds right after tool results: close the turn so the history doesn't
        // end on an unanswered tool result.
        let note = "I stopped searching after several rounds. Try a narrower question.";
        messages.push(json!({"role": "assistant", "content": [{"type": "text", "text": note}]}));
        if reply.text.is_empty() {
            reply.text = note.into();
        }
    }
    state.chat.lock().messages = messages;
    Ok(reply)
}

/// Replaces thumbnails in finished turns with a short note. Only called between questions,
/// so the turn in progress (whose thinking blocks must stay exactly as sent) is untouched.
fn strip_old_images(mut messages: Vec<Value>) -> Vec<Value> {
    for m in messages.iter_mut() {
        if let Some(blocks) = m["content"].as_array_mut() {
            for b in blocks.iter_mut() {
                if b["type"] == "tool_result" {
                    if let Some(inner) = b["content"].as_array_mut() {
                        for ib in inner.iter_mut() {
                            if ib["type"] == "image" {
                                *ib = json!({"type": "text", "text": "[thumbnail shown earlier]"});
                            }
                        }
                    }
                }
            }
        }
    }
    messages
}

// ------------------------------------------------------------------ OpenAI-compatible (Ollama, LM Studio…)

async fn chat_openai(state: &Arc<AppState>, s: &AiSettings, user_text: String) -> Result<ChatReply> {
    let base = check_base_url(&s.openai_base_url)?;
    let key = crate::keys::get("openai");
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(600)).build()?;
    let tools: Vec<Value> = tools_spec()
        .into_iter()
        .map(|t| json!({"type": "function", "function": {"name": t["name"], "description": t["description"], "parameters": t["input_schema"]}}))
        .collect();
    let mut messages = { state.chat.lock().messages.clone() };
    if messages.is_empty() {
        messages.push(json!({"role": "system", "content": SYSTEM_PROMPT}));
    }
    messages.push(json!({"role": "user", "content": user_text}));
    let mut reply = ChatReply::default();
    let url = format!("{}/chat/completions", base.as_str().trim_end_matches('/'));
    let mut finished = false;
    for _round in 0..MAX_ROUNDS {
        emit_status(state, "Thinking…");
        let mut req = client.post(&url).json(&json!({"model": s.openai_model, "messages": messages, "tools": tools}));
        if let Some(k) = &key {
            // check_base_url allows only https, or http on this computer.
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.map_err(|e| anyhow!("Could not reach {url}: {e}"))?;
        let (status, v) = json_reply(resp, "The AI server").await?;
        if !status.is_success() {
            bail!("AI server error ({}): {}", status.as_u16(), v["error"]["message"].as_str().unwrap_or(&v.to_string()));
        }
        let msg = v["choices"][0]["message"].clone();
        messages.push(msg.clone());
        let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
        if calls.is_empty() {
            reply.text = msg["content"].as_str().unwrap_or("").to_string();
            finished = true;
            break;
        }
        let mut images = vec![];
        for c in calls {
            let name = c["function"]["name"].as_str().unwrap_or("");
            let args: Value = match &c["function"]["arguments"] {
                Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                other => other.clone(),
            };
            let (blocks, _err) = tool(state, name, &args, &mut reply, s.allow_images).await;
            let mut text = vec![];
            for b in blocks {
                if b["type"] == "image" {
                    images.push(json!({"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{}", b["source"]["data"].as_str().unwrap_or(""))}}));
                } else if let Some(t) = b["text"].as_str() {
                    text.push(t.to_string());
                }
            }
            messages.push(json!({"role": "tool", "tool_call_id": c["id"], "content": text.join("\n")}));
        }
        if !images.is_empty() {
            let mut content = vec![json!({"type": "text", "text": "Here are the thumbnails you asked for."})];
            content.extend(images);
            messages.push(json!({"role": "user", "content": content}));
        }
    }
    if !finished {
        let note = "I stopped searching after several rounds. Try a narrower question.";
        messages.push(json!({"role": "assistant", "content": note}));
        if reply.text.is_empty() {
            reply.text = note.into();
        }
    }
    // Drop images from history.
    for m in messages.iter_mut() {
        if let Some(a) = m["content"].as_array_mut() {
            a.retain(|b| b["type"] != "image_url");
        }
    }
    state.chat.lock().messages = messages;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_points_are_not_nan() {
        let d = haversine_km((0.0, 0.0), (0.0, 180.0));
        assert!(d.is_finite() && (d - 20015.0).abs() < 5.0, "{d}");
        assert!(haversine_km((89.999999, 0.0), (-89.999999, 180.0)).is_finite());
    }

    #[test]
    fn ai_server_addresses() {
        for ok in ["http://localhost:11434/v1", "http://127.0.0.1:1234/v1", "http://[::1]:8080", "https://api.openai.com/v1", "https://ai.example.org"] {
            assert!(check_base_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://169.254.169.254/latest/meta-data",
            "https://169.254.169.254/",
            "http://[fe80::1]/",
            "http://192.168.1.20:11434/v1",
            "http://api.example.com/v1",
            "file:///etc/passwd",
            "https://user:pw@example.com",
            "https://metadata.google.internal/",
            "not a url",
        ] {
            assert!(check_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn history_is_trimmed_at_question_boundaries() {
        let q = |t: &str| json!({"role": "user", "content": [{"type": "text", "text": t}]});
        let a = json!({"role": "assistant", "content": [{"type": "tool_use", "id": "1"}]});
        let r = json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "1"}]});
        let mut m = vec![];
        for i in 0..10 {
            m.extend([q(&i.to_string()), a.clone(), r.clone()]);
        }
        let t = trim_history(m, 7);
        assert!(t.len() <= 7);
        assert!(is_question(&t[0]), "must start at a question");
        let sys = vec![json!({"role": "system", "content": "s"}), json!({"role": "user", "content": "x"}), json!({"role": "assistant", "content": "y"}), json!({"role": "user", "content": "z"})];
        let t = trim_history(sys, 2);
        assert_eq!(t[0]["role"], "system");
        assert_eq!(t.last().unwrap()["content"], "z");
    }

    #[test]
    fn old_images_become_notes() {
        let m = vec![json!({"role": "user", "content": [{"type": "tool_result", "content": [{"type": "image", "source": {}}]}]})];
        let out = strip_old_images(m);
        assert_eq!(out[0]["content"][0]["content"][0]["type"], "text");
    }

    #[test]
    fn one_question_at_a_time() {
        let first = Turn::start().unwrap();
        assert!(Turn::start().is_err());
        drop(first);
        assert!(Turn::start().is_ok());
    }
}
