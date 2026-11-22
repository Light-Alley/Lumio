//! Upload photos to SmugMug (API v2).
//!
//! SmugMug only offers OAuth 1.0a. Every app needs its own API key and secret, which
//! anyone with a SmugMug account can get free at https://api.smugmug.com/api/developer/apply.
//! Because Lumio is open source a key built into the app wouldn't stay secret, so each
//! user pastes their own key once (see docs/smugmug.md). Sign-in then works like any
//! other app: SmugMug opens in the browser, the user clicks "Authorize", and SmugMug
//! shows a six-digit code to paste back into Lumio. The key, secret and the access token
//! SmugMug hands back are kept in the system keychain; access tokens don't expire until
//! the user revokes them on SmugMug.
//!
//! Uploads go one file at a time to upload.smugmug.com, streamed from disk so large
//! videos don't have to fit in memory. Files SmugMug can't show as they are (RAW, TIFF,
//! WebP…) are sent as full-size JPEG copies made by [`webexport::render_jpeg`].

use crate::db::{self, Photo};
use crate::metadata;
use crate::state::AppState;
use crate::webexport::{self, ExportOptions};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::Sha1;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;

const API: &str = "https://api.smugmug.com";
const REQUEST_TOKEN_URL: &str = "https://api.smugmug.com/services/oauth/1.0a/getRequestToken";
const AUTHORIZE_URL: &str = "https://api.smugmug.com/services/oauth/1.0a/authorize";
const ACCESS_TOKEN_URL: &str = "https://api.smugmug.com/services/oauth/1.0a/getAccessToken";
const UPLOAD_URL: &str = "https://upload.smugmug.com/";
pub const APPLY_URL: &str = "https://api.smugmug.com/api/developer/apply";
/// Name of the keychain entry holding everything SmugMug-related, as one JSON value.
const KEY_NAME: &str = "smugmug";

/// What SmugMug can display as uploaded. Everything else becomes a JPEG copy.
const SEND_AS_IS: &[&str] = &["jpg", "jpeg", "png", "gif", "heic", "heif"];

// ------------------------------------------------------------------ credentials

#[derive(Serialize, Deserialize, Default, Clone)]
struct Creds {
    key: String,
    secret: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    token_secret: String,
    #[serde(default)]
    nickname: String,
}

impl Creds {
    fn connected(&self) -> bool {
        !self.token.is_empty() && !self.token_secret.is_empty()
    }
}

fn load() -> Creds {
    crate::keys::get(KEY_NAME).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(c: &Creds) -> Result<()> {
    if c.key.is_empty() && c.secret.is_empty() {
        return crate::keys::set(KEY_NAME, "");
    }
    crate::keys::set(KEY_NAME, &serde_json::to_string(c)?)
}

/// The request token waiting for the user to paste SmugMug's six-digit code.
static PENDING: Mutex<Option<(String, String)>> = Mutex::new(None);
/// Web addresses SmugMug gave this session (the approval page, galleries): the only
/// ones besides APPLY_URL that `smugmug_open` will open. Galleries can live on a custom
/// domain, so a fixed host list wouldn't do.
static OPENABLE: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn remember(url: &str) {
    if url.starts_with("https://") {
        OPENABLE.lock().get_or_insert_with(HashSet::new).insert(url.to_string());
    }
}

fn may_open(url: &str) -> bool {
    let https = reqwest::Url::parse(url).map(|u| u.scheme() == "https").unwrap_or(false);
    https && (url == APPLY_URL || OPENABLE.lock().as_ref().is_some_and(|s| s.contains(url)))
}

/// An API address for `path`, which SmugMug's own replies supply ("/api/v2/…"). Only
/// paths on api.smugmug.com are accepted, so a reply naming another host can never
/// receive Lumio's signed request.
fn api_url(path: &str) -> Result<reqwest::Url> {
    if !path.starts_with('/') || path.starts_with("//") || path.contains('\\') {
        bail!("SmugMug sent an unexpected address.");
    }
    let url = reqwest::Url::parse(API)?.join(path)?;
    if url.scheme() != "https" || url.host_str() != Some("api.smugmug.com") {
        bail!("SmugMug sent an unexpected address.");
    }
    Ok(url)
}
static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
pub struct Status {
    pub has_app_key: bool,
    pub connected: bool,
    pub nickname: String,
    pub apply_url: &'static str,
}

pub fn status() -> Status {
    let c = load();
    Status {
        has_app_key: !c.key.is_empty() && !c.secret.is_empty(),
        connected: c.connected(),
        nickname: c.nickname,
        apply_url: APPLY_URL,
    }
}

/// Save the user's own API key and secret. Changing them signs out.
pub fn set_app_key(key: &str, secret: &str) -> Result<()> {
    let (key, secret) = (key.trim(), secret.trim());
    if key.is_empty() != secret.is_empty() {
        bail!("Please paste both the API key and the secret.");
    }
    save(&Creds { key: key.into(), secret: secret.into(), ..Default::default() })
}

pub fn sign_out() -> Result<()> {
    let c = load();
    save(&Creds { key: c.key, secret: c.secret, ..Default::default() })
}

// ------------------------------------------------------------------ OAuth 1.0a

/// RFC 3986 percent-encoding, as OAuth 1.0a requires (only A-Z a-z 0-9 - . _ ~ left as is).
const OAUTH_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, OAUTH_SET).to_string()
}

/// 16 random bytes from the operating system, as OAuth asks for an unpredictable nonce.
fn nonce() -> Result<String> {
    crate::secure::random_hex(16)
}

/// Build the `Authorization: OAuth …` header for a request. Query parameters are part of
/// the signature; JSON and file bodies are not (OAuth only signs form-encoded bodies).
fn sign(
    method: &str,
    url: &reqwest::Url,
    consumer: (&str, &str),
    token: Option<(&str, &str)>,
    extra: &[(&str, &str)],
    timestamp: u64,
    nonce: &str,
) -> String {
    let ts = timestamp.to_string();
    let mut oauth: Vec<(String, String)> = vec![
        ("oauth_consumer_key".into(), consumer.0.into()),
        ("oauth_nonce".into(), nonce.into()),
        ("oauth_signature_method".into(), "HMAC-SHA1".into()),
        ("oauth_timestamp".into(), ts),
        ("oauth_version".into(), "1.0".into()),
    ];
    if let Some((t, _)) = token {
        oauth.push(("oauth_token".into(), t.into()));
    }
    for (k, v) in extra {
        oauth.push(((*k).into(), (*v).into()));
    }
    let mut all: Vec<(String, String)> = oauth.iter().map(|(k, v)| (enc(k), enc(v))).collect();
    all.extend(url.query_pairs().map(|(k, v)| (enc(&k), enc(&v))));
    all.sort();
    let params = all.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let mut base_url = url.clone();
    base_url.set_query(None);
    base_url.set_fragment(None);
    let base = format!("{}&{}&{}", method.to_ascii_uppercase(), enc(base_url.as_str()), enc(&params));
    let key = format!("{}&{}", enc(consumer.1), enc(token.map(|t| t.1).unwrap_or("")));
    let mut mac = Hmac::<Sha1>::new_from_slice(key.as_bytes()).expect("any key length works");
    mac.update(base.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    oauth.push(("oauth_signature".into(), sig));
    let fields = oauth.iter().map(|(k, v)| format!("{}=\"{}\"", enc(k), enc(v))).collect::<Vec<_>>().join(", ");
    format!("OAuth {fields}")
}

fn auth(
    method: &str,
    url: &reqwest::Url,
    c: &Creds,
    token: Option<(&str, &str)>,
    extra: &[(&str, &str)],
) -> Result<String> {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    Ok(sign(method, url, (&c.key, &c.secret), token, extra, ts, &nonce()?))
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!("Lumio/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn parse_form(s: &str) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://x/?{}", s.trim()))
        .map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
        .unwrap_or_default()
}

fn form_get(pairs: &[(String, String)], k: &str) -> Option<String> {
    pairs.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone())
}

/// Step 1: get a request token and return the page where the user approves Lumio.
pub async fn begin_sign_in() -> Result<String> {
    let c = load();
    if c.key.is_empty() {
        bail!("Add your SmugMug API key first.");
    }
    let url = reqwest::Url::parse(REQUEST_TOKEN_URL)?;
    let header = auth("GET", &url, &c, None, &[("oauth_callback", "oob")])?;
    let resp = client()?.get(url).header("Authorization", header).send().await.context("contacting SmugMug")?;
    let code = resp.status();
    let body = resp.text().await?;
    if !code.is_success() {
        log::info!("SmugMug request token failed: {code} {}", crate::secure::redact_body(&body));
        bail!("SmugMug didn't accept the API key and secret. Check that both were copied in full ({}).", code.as_u16());
    }
    let pairs = parse_form(&body);
    let (Some(t), Some(s)) = (form_get(&pairs, "oauth_token"), form_get(&pairs, "oauth_token_secret")) else {
        bail!("SmugMug sent an unexpected reply. Please try again.");
    };
    *PENDING.lock() = Some((t.clone(), s));
    let mut auth_url = reqwest::Url::parse(AUTHORIZE_URL)?;
    auth_url
        .query_pairs_mut()
        .append_pair("oauth_token", &t)
        .append_pair("Access", "Full")
        .append_pair("Permissions", "Add");
    remember(auth_url.as_str());
    Ok(auth_url.to_string())
}

/// Step 2: trade the six-digit code for a lasting access token.
pub async fn finish_sign_in(code: &str) -> Result<Status> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.is_empty() {
        bail!("Please enter the code SmugMug showed you.");
    }
    let Some((rt, rs)) = PENDING.lock().clone() else {
        bail!("Please click \"Sign in to SmugMug\" again; the sign-in request has expired.");
    };
    let mut c = load();
    let url = reqwest::Url::parse(ACCESS_TOKEN_URL)?;
    let header = auth("GET", &url, &c, Some((&rt, &rs)), &[("oauth_verifier", &code)])?;
    let resp = client()?.get(url).header("Authorization", header).send().await.context("contacting SmugMug")?;
    let status_code = resp.status();
    let body = resp.text().await?;
    if !status_code.is_success() {
        log::info!("SmugMug access token failed: {status_code} {}", crate::secure::redact_body(&body));
        bail!("SmugMug didn't accept that code. Codes only work once and expire quickly, so click \"Sign in to SmugMug\" again for a new one.");
    }
    let pairs = parse_form(&body);
    let (Some(t), Some(s)) = (form_get(&pairs, "oauth_token"), form_get(&pairs, "oauth_token_secret")) else {
        bail!("SmugMug sent an unexpected reply. Please try again.");
    };
    *PENDING.lock() = None;
    c.token = t;
    c.token_secret = s;
    c.nickname = String::new();
    if let Ok(u) = auth_user(&c).await {
        c.nickname = u.nickname;
    }
    save(&c)?;
    Ok(status())
}

// ------------------------------------------------------------------ API calls

async fn api(c: &Creds, method: &str, path: &str, query: &[(&str, &str)], body: Option<Value>) -> Result<Value> {
    if !c.connected() {
        bail!("Sign in to SmugMug first.");
    }
    let mut url = api_url(path)?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("_verbosity", "1");
        for (k, v) in query {
            q.append_pair(k, v);
        }
    }
    let header = auth(method, &url, c, Some((&c.token, &c.token_secret)), &[])?;
    let http = client()?;
    let mut req = match method {
        "POST" => http.post(url.clone()),
        _ => http.get(url.clone()),
    }
    .timeout(Duration::from_secs(60))
    .header("Authorization", header)
    .header("Accept", "application/json");
    if let Some(b) = body {
        req = req.header("Content-Type", "application/json").body(serde_json::to_vec(&b)?);
    }
    let resp = req.send().await.context("contacting SmugMug")?;
    let code = resp.status();
    let text = resp.text().await?;
    if code.as_u16() == 401 {
        log::info!(
            "SmugMug 401 for {}: {}",
            crate::secure::redact_url(url.as_str()),
            crate::secure::redact_body(&text)
        );
        bail!("SmugMug no longer accepts Lumio's sign-in. Please sign out and sign in again.");
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !code.is_success() {
        log::info!(
            "SmugMug {code} for {}: {}",
            crate::secure::redact_url(url.as_str()),
            crate::secure::redact_body(&text)
        );
        let msg = v["Message"].as_str().unwrap_or("").to_string();
        bail!(
            "SmugMug said: {} ({})",
            if msg.is_empty() { code.canonical_reason().unwrap_or("error") } else { &msg },
            code.as_u16()
        );
    }
    Ok(v["Response"].clone())
}

struct AuthUser {
    nickname: String,
    node: Option<String>,
    albums: Option<String>,
}

async fn auth_user(c: &Creds) -> Result<AuthUser> {
    let r = api(c, "GET", "/api/v2!authuser", &[], None).await?;
    let u = &r["User"];
    let nickname = u["NickName"].as_str().unwrap_or_default().to_string();
    if nickname.is_empty() {
        bail!("SmugMug didn't say which account this is.");
    }
    let uri =
        |k: &str| u["Uris"][k].as_str().map(String::from).or_else(|| u["Uris"][k]["Uri"].as_str().map(String::from));
    Ok(AuthUser { node: uri("Node"), albums: uri("UserAlbums"), nickname })
}

#[derive(Serialize, Clone)]
pub struct Album {
    pub name: String,
    pub uri: String,
    pub web_uri: String,
    /// Where it sits on the site, e.g. "/Travel/Iceland-2026".
    pub path: String,
    pub privacy: String,
    pub image_count: Option<i64>,
    pub updated: String,
}

fn album_from(v: &Value) -> Option<Album> {
    let uri = v["Uri"].as_str()?.to_string();
    Some(Album {
        name: v["Name"].as_str().or(v["Title"].as_str()).unwrap_or("Untitled").to_string(),
        web_uri: v["WebUri"].as_str().unwrap_or_default().to_string(),
        path: v["UrlPath"].as_str().unwrap_or_default().to_string(),
        privacy: v["Privacy"].as_str().unwrap_or_default().to_string(),
        image_count: v["ImageCount"].as_i64(),
        updated: v["ImagesLastUpdated"].as_str().or(v["LastUpdated"].as_str()).unwrap_or_default().to_string(),
        uri,
    })
}

/// The signed-in user's galleries, most recently changed first.
pub async fn albums() -> Result<Vec<Album>> {
    let mut c = load();
    let me = auth_user(&c).await?;
    if c.nickname != me.nickname {
        c.nickname = me.nickname.clone();
        let _ = save(&c);
    }
    let path = me.albums.unwrap_or_else(|| format!("/api/v2/user/{}!albums", me.nickname));
    let mut out = vec![];
    let mut start = 1;
    for _page in 0..50 {
        let s = start.to_string();
        let r = api(&c, "GET", &path, &[("start", &s), ("count", "100")], None).await?;
        let list = r["Album"].as_array().cloned().unwrap_or_default();
        out.extend(list.iter().filter_map(album_from));
        for a in &out {
            remember(&a.web_uri);
        }
        let total = r["Pages"]["Total"].as_i64().unwrap_or(0);
        start += list.len() as i64;
        if list.is_empty() || start > total {
            break;
        }
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    Ok(out)
}

/// A web-address-friendly version of a gallery name, e.g. "Iceland 2026!" → "Iceland-2026".
pub fn url_name(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            let first = ch.next().unwrap().to_ascii_uppercase();
            std::iter::once(first).chain(ch).collect()
        })
        .collect();
    let mut s = words.join("-");
    s.truncate(60);
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() || !s.starts_with(|c: char| c.is_ascii_uppercase()) {
        format!("Gallery-{s}").trim_end_matches('-').to_string()
    } else {
        s
    }
}

/// Make a new gallery at the top level of the user's site.
pub async fn create_album(name: &str, privacy: &str) -> Result<Album> {
    let name = name.trim();
    if name.is_empty() {
        bail!("Please give the gallery a name.");
    }
    let privacy = match privacy {
        "Public" | "Unlisted" | "Private" => privacy,
        _ => "Unlisted",
    };
    let c = load();
    let me = auth_user(&c).await?;
    let Some(node) = me.node else {
        bail!("This SmugMug account doesn't support creating galleries from other apps. Please create the gallery on smugmug.com, then pick it here.");
    };
    let children = format!("{node}!children");
    let base = url_name(name);
    let mut last_err = anyhow!("could not create the gallery");
    for attempt in 0..3 {
        let un = if attempt == 0 { base.clone() } else { format!("{base}-{}", attempt + 1) };
        let body = json!({"Type": "Album", "Name": name, "UrlName": un, "Privacy": privacy});
        match api(&c, "POST", &children, &[], Some(body)).await {
            Ok(r) => {
                let n = &r["Node"];
                let album_uri = n["Uris"]["Album"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| n["Uris"]["Album"]["Uri"].as_str().map(String::from));
                let Some(uri) = album_uri else {
                    bail!("SmugMug created the gallery but didn't say where it is. Please pick it from the list.")
                };
                remember(n["WebUri"].as_str().unwrap_or_default());
                return Ok(Album {
                    name: name.into(),
                    uri,
                    web_uri: n["WebUri"].as_str().unwrap_or_default().into(),
                    path: n["UrlPath"].as_str().unwrap_or_default().into(),
                    privacy: privacy.into(),
                    image_count: Some(0),
                    updated: String::new(),
                });
            }
            Err(e) => {
                // A gallery with the same web address already exists: try the next one.
                let m = e.to_string();
                last_err = e;
                if !(m.contains("409") || m.to_lowercase().contains("urlname") || m.to_lowercase().contains("already"))
                {
                    break;
                }
            }
        }
    }
    Err(last_err)
}

// ------------------------------------------------------------------ uploading

#[derive(Deserialize, Clone, Debug)]
pub struct UploadOptions {
    /// "original": send files as they are (RAW etc. become full-size JPEGs).
    /// "web": send JPEGs resized to `long_edge`.
    pub size: String,
    pub long_edge: Option<u32>,
}

#[derive(Serialize, Default)]
pub struct UploadReport {
    pub done: usize,
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
    pub web_uri: String,
}

pub fn cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}

fn content_type(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "mpg" | "mpeg" => "video/mpeg",
        "3gp" => "video/3gpp",
        "mts" | "m2ts" => "video/mp2t",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        _ => "application/octet-stream",
    }
}

/// Header values can't hold line breaks; keep the rest (UTF-8 is allowed as raw bytes).
fn hv(s: &str) -> Option<HeaderValue> {
    let one_line: String = s.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return None;
    }
    HeaderValue::from_bytes(one_line.as_bytes()).ok()
}

struct Prepared {
    file: PathBuf,
    name: String,
    ctype: &'static str,
    temp: bool,
}

fn prepare(state: &AppState, p: &Photo, opts: &UploadOptions) -> Result<Prepared> {
    let src = Path::new(&p.path);
    if !src.exists() {
        bail!("not available (is its drive connected?)");
    }
    let ext = p.ext.to_ascii_lowercase();
    let web = opts.size == "web" && !metadata::is_video(&ext);
    if !web && (metadata::is_video(&ext) || SEND_AS_IS.contains(&ext.as_str())) {
        return Ok(Prepared {
            file: src.to_path_buf(),
            name: p.filename.clone(),
            ctype: content_type(&ext),
            temp: false,
        });
    }
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| format!("photo-{}", p.id));
    let tmp = state.tmp_dir.join(format!("smugmug-{}-{}.jpg", p.id, crate::secure::random_hex(8)?));
    let eo = ExportOptions {
        long_edge: if web { opts.long_edge.or(Some(3000)) } else { None },
        quality: if web { 88 } else { 95 },
        strip_location: false,
    };
    webexport::render_jpeg(state, p, &eo, &tmp)?;
    Ok(Prepared { file: tmp, name: format!("{stem}.jpg"), ctype: "image/jpeg", temp: true })
}

async fn upload_one(
    c: &Creds,
    album_uri: &str,
    p: &Photo,
    f: &Prepared,
    progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<()> {
    let size = tokio::fs::metadata(&f.file).await?.len();
    let url = reqwest::Url::parse(UPLOAD_URL)?;
    let mut h = HeaderMap::new();
    h.insert("Authorization", HeaderValue::from_str(&auth("POST", &url, c, Some((&c.token, &c.token_secret)), &[])?)?);
    h.insert("Content-Length", HeaderValue::from(size));
    h.insert("Content-Type", HeaderValue::from_static(f.ctype));
    h.insert("X-Smug-AlbumUri", HeaderValue::from_str(album_uri)?);
    h.insert("X-Smug-ResponseType", HeaderValue::from_static("JSON"));
    h.insert("X-Smug-Version", HeaderValue::from_static("v2"));
    let mut put = |k: &'static str, v: Option<&str>| {
        if let Some(v) = v.and_then(hv) {
            h.insert(HeaderName::from_static(k), v);
        }
    };
    put("x-smug-filename", Some(&f.name));
    put("x-smug-title", p.title.as_deref());
    put("x-smug-caption", p.caption.as_deref());
    let kw = p.keywords.join("; ");
    put("x-smug-keywords", Some(&kw));

    // Stream the file, reporting bytes as they go and stopping if the user cancels.
    let file = tokio::fs::File::open(&f.file).await?;
    let sent = Arc::new(AtomicU64::new(0));
    let sent2 = sent.clone();
    let stream = tokio_util::io::ReaderStream::with_capacity(file, 256 * 1024).map(move |chunk| {
        if CANCEL.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
        if let Ok(b) = &chunk {
            sent2.fetch_add(b.len() as u64, Ordering::Relaxed);
        }
        chunk
    });
    let http = client()?;
    let req = http.post(url).headers(h).body(reqwest::Body::wrap_stream(stream)).send();
    tokio::pin!(req);
    let resp = loop {
        tokio::select! {
            r = &mut req => break r,
            _ = tokio::time::sleep(Duration::from_millis(250)) => progress(sent.load(Ordering::Relaxed), size),
        }
    };
    progress(size, size);
    if CANCEL.load(Ordering::Relaxed) {
        bail!("cancelled");
    }
    let resp = resp.context("upload interrupted")?;
    let code = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if code.is_success() && v["stat"].as_str() == Some("ok") {
        return Ok(());
    }
    log::info!("SmugMug upload of {} failed: {code} {}", p.filename, crate::secure::redact_body(&text));
    let msg =
        v["message"].as_str().or(v["Message"].as_str()).unwrap_or(code.canonical_reason().unwrap_or("upload failed"));
    if code.as_u16() == 401 {
        bail!("SmugMug no longer accepts Lumio's sign-in; please sign in again");
    }
    Err(anyhow!("{msg}"))
}

pub async fn upload(
    state_arc: Arc<AppState>,
    ids: Vec<i64>,
    album_uri: String,
    web_uri: String,
    opts: UploadOptions,
) -> Result<UploadReport> {
    CANCEL.store(false, Ordering::Relaxed);
    if !album_uri.starts_with("/api/v2/album/") || album_uri.contains(['\r', '\n']) {
        bail!("Please pick a gallery from the list.");
    }
    let state: &AppState = &state_arc;
    let c = load();
    if !c.connected() {
        bail!("Sign in to SmugMug first.");
    }
    let photos = db::get_photos(&state.conn.lock(), &ids)?;
    let total = photos.len();
    let mut report = UploadReport { web_uri, ..Default::default() };
    for (i, p) in photos.iter().enumerate() {
        if CANCEL.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let emit = |bytes: u64, size: u64| {
            let _ = state.app.emit(
                "smugmug-progress",
                json!({ "done": i, "total": total, "current": p.filename, "bytes": bytes, "size": size }),
            );
        };
        emit(0, 0);
        let st = state_arc.clone();
        let pc = p.clone();
        let o = opts.clone();
        // Decoding and resizing are CPU work; keep them off the async threads.
        let prepared = tauri::async_runtime::spawn_blocking(move || prepare(&st, &pc, &o))
            .await
            .map_err(|e| anyhow!("{e}"))
            .and_then(|r| r);
        let result = match prepared {
            Ok(f) => {
                let mut r = upload_one(&c, &album_uri, p, &f, &emit).await;
                // One retry for network hiccups and SmugMug being busy.
                if let Err(e) = &r {
                    let m = e.to_string();
                    if !CANCEL.load(Ordering::Relaxed)
                        && !m.contains("sign in")
                        && (m.contains("interrupted") || m.contains("Too Many") || m.contains("Service Unavailable"))
                    {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        r = upload_one(&c, &album_uri, p, &f, &emit).await;
                    }
                }
                if f.temp {
                    let _ = std::fs::remove_file(&f.file);
                }
                r
            }
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => report.done += 1,
            Err(_) if CANCEL.load(Ordering::Relaxed) => {
                report.cancelled = true;
                break;
            }
            Err(e) => {
                let m = format!("{e:#}");
                let stop = m.contains("sign in");
                report.failed.push((p.filename.clone(), m));
                if stop {
                    break;
                }
            }
        }
    }
    let _ = state
        .app
        .emit("smugmug-progress", json!({ "done": total, "total": total, "current": "", "bytes": 0, "size": 0 }));
    Ok(report)
}

/// The commands the user interface calls.
pub mod cmd {
    use super::*;
    use tauri::State;

    type S<'a> = State<'a, Arc<AppState>>;
    type R<T> = Result<T, String>;
    fn e(x: anyhow::Error) -> String {
        format!("{x:#}")
    }

    #[tauri::command]
    pub fn smugmug_status() -> Status {
        status()
    }

    #[tauri::command]
    pub fn smugmug_set_app_key(key: String, secret: String) -> R<Status> {
        set_app_key(&key, &secret).map_err(e)?;
        Ok(status())
    }

    /// Opens SmugMug's approval page in the browser and also returns its address,
    /// so the window can offer it as a link if the browser didn't open.
    #[tauri::command]
    pub async fn smugmug_begin_sign_in() -> R<String> {
        let url = begin_sign_in().await.map_err(e)?;
        if let Err(x) = tauri_plugin_opener::open_url(&url, None::<&str>) {
            log::info!("could not open the browser: {x}");
        }
        Ok(url)
    }

    #[tauri::command]
    pub async fn smugmug_finish_sign_in(code: String) -> R<Status> {
        finish_sign_in(&code).await.map_err(e)
    }

    #[tauri::command]
    pub fn smugmug_sign_out(forget_key: bool) -> R<Status> {
        if forget_key { set_app_key("", "") } else { sign_out() }.map_err(e)?;
        Ok(status())
    }

    #[tauri::command]
    pub async fn smugmug_albums() -> R<Vec<Album>> {
        albums().await.map_err(e)
    }

    #[tauri::command]
    pub async fn smugmug_create_album(name: String, privacy: String) -> R<Album> {
        create_album(&name, &privacy).await.map_err(e)
    }

    #[tauri::command]
    pub async fn smugmug_upload(
        state: S<'_>,
        ids: Vec<i64>,
        album_uri: String,
        web_uri: String,
        opts: UploadOptions,
    ) -> R<UploadReport> {
        upload(state.inner().clone(), ids, album_uri, web_uri, opts).await.map_err(e)
    }

    #[tauri::command]
    pub fn smugmug_cancel() {
        cancel();
    }

    /// Open a SmugMug page in the browser: the developer sign-up page, the approval page
    /// from the current sign-in, or a gallery SmugMug itself named in this session.
    #[tauri::command]
    pub fn smugmug_open(url: String) -> R<()> {
        if !may_open(&url) {
            return Err("Lumio only opens SmugMug pages it got from SmugMug.".into());
        }
        tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|x| x.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_names() {
        assert_eq!(url_name("Iceland 2026!"), "Iceland-2026");
        assert_eq!(url_name("  wedding  of  sam & alex "), "Wedding-Of-Sam-Alex");
        assert_eq!(url_name("2026 trip"), "Gallery-2026-Trip");
        assert_eq!(url_name("Café"), "Caf");
        assert_eq!(url_name("日本"), "Gallery");
    }

    #[test]
    fn api_requests_stay_on_smugmug() {
        assert_eq!(api_url("/api/v2!authuser").unwrap().as_str(), "https://api.smugmug.com/api/v2!authuser");
        for bad in ["https://attacker.example/api", "//attacker.example/x", "api/v2", "/\\attacker.example", ""] {
            assert!(api_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_known_pages_open() {
        assert!(may_open(APPLY_URL));
        assert!(!may_open("https://example.com/"));
        assert!(!may_open("file:///etc/passwd"));
        remember("https://photos.example.com/Travel/Iceland");
        assert!(may_open("https://photos.example.com/Travel/Iceland"));
        remember("http://insecure.example.com/");
        assert!(!may_open("http://insecure.example.com/"));
    }

    #[test]
    fn nonces_are_random() {
        let (a, b) = (nonce().unwrap(), nonce().unwrap());
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn rfc3986_encoding() {
        assert_eq!(enc("a b+c/~d.e_f-g!"), "a%20b%2Bc%2F~d.e_f-g%21");
    }

    /// Twitter's widely used OAuth 1.0a worked example: known inputs, known signature.
    #[test]
    fn signature_matches_known_example() {
        let url = reqwest::Url::parse("https://api.twitter.com/1.1/statuses/update.json?include_entities=true&status=Hello%20Ladies%20%2B%20Gentlemen%2C%20a%20signed%20OAuth%20request%21").unwrap();
        let h = sign(
            "POST",
            &url,
            ("xvz1evFS4wEEPTGEFPHBog", "kAcSOqF21Fu85e7zjz7ZN2U4ZRhfV3WpwPAoE3Z7kBw"),
            Some(("370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb", "LswwdoUaIvS8ltyTt5jkRh4J50vUPVVHtR2YPi5kE")),
            &[],
            1318622958,
            "kYjzVBB8Y0ZFabxSWbWovY3uYSQ2pTgmZeNu2VS4cg",
        );
        assert!(h.contains("oauth_signature=\"hCtSmYh%2BiHYCEqBWrE7C7hYmtUk%3D\""), "{h}");
    }
}
