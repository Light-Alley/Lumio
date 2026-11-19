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
use tauri::Emitter;

pub const ANTHROPIC_VERSION: &str = "2026-10-21";
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-5-5";
const KEYRING_SERVICE: &str = "Lumio Photo Manager";

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
        has_anthropic_key: get_key_inner(&conn, "anthropic").is_some(),
        has_openai_key: get_key_inner(&conn, "openai").is_some(),
    }
}

pub(crate) fn get_key_inner(conn: &rusqlite::Connection, which: &str) -> Option<String> {
    if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, which) {
        if let Ok(k) = e.get_password() {
            if !k.is_empty() {
                return Some(k);
            }
        }
    }
    // Fallback for systems without a usable keychain.
    db::get_setting(conn, &format!("key_{which}")).filter(|k| !k.is_empty())
}

pub fn set_key(state: &AppState, which: &str, key: &str) -> Result<()> {
    let conn = state.conn.lock();
    let key = key.trim();
    let stored = keyring::Entry::new(KEYRING_SERVICE, which).and_then(|e| {
        if key.is_empty() {
            match e.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(x) => Err(x),
            }
        } else {
            e.set_password(key)
        }
    });
    if stored.is_ok() {
        conn.execute("DELETE FROM settings WHERE key = ?1", [format!("key_{which}")])?;
    } else {
        db::set_setting(&conn, &format!("key_{which}"), key)?;
    }
    Ok(())
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

pub async fn chat(state: &AppState, user_text: String) -> Result<ChatReply> {
    let s = settings(state);
    {
        let mut c = state.chat.lock();
        if c.provider != s.provider {
            c.messages.clear();
            c.provider = s.provider.clone();
        }
    }
    let r = if s.provider == "openai" { chat_openai(state, &s, user_text).await } else { chat_anthropic(state, &s, user_text).await };
    emit_status(state, "");
    r
}

// ------------------------------------------------------------------ tools

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
    6371.0 * 2.0 * h.sqrt().asin()
}

fn search(state: &AppState, i: &Value) -> Result<Value> {
    let mut wh: Vec<String> = vec![];
    let mut args: Vec<rusqlite::types::Value> = vec![];
    let hay = "lower(filename || ' ' || ifnull(title,'') || ' ' || ifnull(caption,'') || ' ' || ifnull(keywords,'') || ' ' || ifnull(place,'') || ' ' || folder || ' ' || ifnull(make,'') || ' ' || ifnull(model,'') || ' ' || ifnull(lens,''))";
    if let Some(t) = i["text"].as_str() {
        for w in t.split_whitespace() {
            args.push(format!("%{}%", w.to_lowercase()).into());
            wh.push(format!("{hay} LIKE ?{}", args.len()));
        }
    }
    if let Some(a) = i["any_text"].as_array() {
        let mut ors = vec![];
        for w in a.iter().filter_map(|v| v.as_str()) {
            args.push(format!("%{}%", w.to_lowercase()).into());
            ors.push(format!("{hay} LIKE ?{}", args.len()));
        }
        if !ors.is_empty() {
            wh.push(format!("({})", ors.join(" OR ")));
        }
    }
    if let Some(a) = i["keywords"].as_array() {
        for k in a.iter().filter_map(|v| v.as_str()) {
            args.push(format!("%|{}|%", k.to_lowercase()).into());
            wh.push(format!("lower(ifnull(keywords,'')) LIKE ?{}", args.len()));
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
        args.push(format!("%{}%", c.to_lowercase()).into());
        wh.push(format!("lower(ifnull(make,'') || ' ' || ifnull(model,'')) LIKE ?{}", args.len()));
    }
    if let Some(c) = i["lens"].as_str() {
        args.push(format!("%{}%", c.to_lowercase()).into());
        wh.push(format!("lower(ifnull(lens,'')) LIKE ?{}", args.len()));
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
    let sql = format!(
        "SELECT {} FROM photos {} ORDER BY {order}",
        db::PHOTO_COLS,
        if wh.is_empty() { String::new() } else { format!("WHERE {}", wh.join(" AND ")) }
    );
    let conn = state.conn.lock();
    let mut st = conn.prepare(&sql)?;
    let mut rows: Vec<db::Photo> = st.query_map(params_from_iter(args.iter()), db::photo_from_row)?.filter_map(|r| r.ok()).collect();
    if let Some((lat, lon, r)) = near {
        rows.retain(|p| match (p.lat, p.lon) {
            (Some(a), Some(b)) => haversine_km((lat, lon), (a, b)) <= r,
            _ => false,
        });
    }
    let total = rows.len();
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

async fn chat_anthropic(state: &AppState, s: &AiSettings, user_text: String) -> Result<ChatReply> {
    let key = { get_key_inner(&state.conn.lock(), "anthropic") }
        .ok_or_else(|| anyhow!("Add your Anthropic API key in Settings → AI to use Claude."))?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build()?;
    let mut messages = { state.chat.lock().messages.clone() };
    messages.push(json!({"role": "user", "content": [{"type": "text", "text": user_text}]}));
    let mut reply = ChatReply::default();
    for _round in 0..12 {
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
        let status = resp.status();
        let v: Value = resp.json().await?;
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
            break;
        }
        if stop != "tool_use" {
            reply.text = text;
            break;
        }
        let mut results = vec![];
        for b in content.as_array().into_iter().flatten().filter(|b| b["type"] == "tool_use") {
            let (blocks, is_error) = run_tool(state, b["name"].as_str().unwrap_or(""), &b["input"], &mut reply, s.allow_images);
            results.push(json!({"type": "tool_result", "tool_use_id": b["id"], "content": blocks, "is_error": is_error}));
        }
        messages.push(json!({"role": "user", "content": results}));
        if !text.is_empty() {
            reply.text = text;
        }
    }
    state.chat.lock().messages = messages;
    Ok(reply)
}

/// Not used for Claude: its history must stay append-only so earlier thinking blocks
/// remain valid. Kept for reference.
#[allow(dead_code)]
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

async fn chat_openai(state: &AppState, s: &AiSettings, user_text: String) -> Result<ChatReply> {
    let key = { get_key_inner(&state.conn.lock(), "openai") };
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
    let url = format!("{}/chat/completions", s.openai_base_url.trim_end_matches('/'));
    for _round in 0..12 {
        emit_status(state, "Thinking…");
        let mut req = client.post(&url).json(&json!({"model": s.openai_model, "messages": messages, "tools": tools}));
        if let Some(k) = &key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.map_err(|e| anyhow!("Could not reach {url}: {e}"))?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if !status.is_success() {
            bail!("AI server error ({}): {}", status.as_u16(), v["error"]["message"].as_str().unwrap_or(&v.to_string()));
        }
        let msg = v["choices"][0]["message"].clone();
        messages.push(msg.clone());
        let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
        if calls.is_empty() {
            reply.text = msg["content"].as_str().unwrap_or("").to_string();
            break;
        }
        let mut images = vec![];
        for c in calls {
            let name = c["function"]["name"].as_str().unwrap_or("");
            let args: Value = match &c["function"]["arguments"] {
                Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                other => other.clone(),
            };
            let (blocks, _err) = run_tool(state, name, &args, &mut reply, s.allow_images);
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
    // Drop images from history.
    for m in messages.iter_mut() {
        if let Some(a) = m["content"].as_array_mut() {
            a.retain(|b| b["type"] != "image_url");
        }
    }
    state.chat.lock().messages = messages;
    Ok(reply)
}
