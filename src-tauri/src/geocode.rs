//! Place search for the map, with OpenStreetMap's Nominatim service.
//!
//! Nominatim's usage policy allows at most one request per second from an application,
//! with an identifying User-Agent, so searches are spaced out here whatever the window
//! asks for.

use serde::Serialize;
use std::time::{Duration, Instant};

const MIN_GAP: Duration = Duration::from_millis(1100);

/// When the last search was sent.
static LAST: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);

#[derive(Serialize)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// How long to wait before the next request may go, given when the last one went.
fn wait_for(last: Option<Instant>, now: Instant) -> Duration {
    last.map(|t| MIN_GAP.saturating_sub(now.saturating_duration_since(t))).unwrap_or_default()
}

fn places(v: &serde_json::Value) -> Vec<Place> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(Place {
                name: p["display_name"].as_str()?.to_string(),
                lat: p["lat"].as_str()?.parse().ok()?,
                lon: p["lon"].as_str()?.parse().ok()?,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn geocode(query: String) -> Result<Vec<Place>, String> {
    let query = query.trim().chars().take(200).collect::<String>();
    if query.is_empty() {
        return Ok(vec![]);
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("Lumio/", env!("CARGO_PKG_VERSION"), " (open-source photo manager)"))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    // One request at a time, at least MIN_GAP apart.
    let mut last = LAST.lock().await;
    tokio::time::sleep(wait_for(*last, Instant::now())).await;
    *last = Some(Instant::now());
    let resp = client
        .get("https://nominatim.openstreetmap.org/search")
        .query(&[("q", query.as_str()), ("format", "jsonv2"), ("limit", "6")])
        .send()
        .await
        .map_err(|e| format!("Place search failed: {e}"))?;
    drop(last);
    let status = resp.status();
    let body = resp.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Place search is not available right now (OpenStreetMap answered {status})."));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| "Place search sent an answer Lumio couldn't read.".to_string())?;
    Ok(places(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_are_at_least_a_second_apart() {
        let now = Instant::now();
        assert_eq!(wait_for(None, now), Duration::ZERO);
        assert!(wait_for(Some(now), now) >= Duration::from_secs(1));
        let long_ago = now.checked_sub(Duration::from_secs(5)).unwrap_or(now);
        if long_ago < now {
            assert_eq!(wait_for(Some(long_ago), now), Duration::ZERO);
        }
    }

    #[test]
    fn reads_nominatim_answers() {
        let v = serde_json::json!([
            {"display_name": "Reykjavík, Iceland", "lat": "64.1466", "lon": "-21.9426"},
            {"display_name": "no position"},
        ]);
        let p = places(&v);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "Reykjavík, Iceland");
        assert!(places(&serde_json::json!({"error": "x"})).is_empty());
    }
}
