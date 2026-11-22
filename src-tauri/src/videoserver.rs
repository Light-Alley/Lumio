//! A tiny local web server that streams videos to the player on Linux.
//!
//! On Linux the webview (WebKitGTK) refuses to play media from a custom address such as
//! `photo://` ("FormatError" before GStreamer even sees the file), while the same file
//! plays fine over plain HTTP. So on Linux videos are served from
//! `http://127.0.0.1:<port>/<secret>/<id>` instead. The server only listens on this
//! computer, picks a free port, and answers only requests that carry a secret made fresh
//! each time Lumio starts, so other programs and web pages can't read files through it.
//! macOS and Windows keep using `photo://`, which works there.

use crate::state::AppState;
use crate::{db, metadata, pending};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Most connections served at once; more are closed straight away.
const MAX_CONNECTIONS: usize = 16;
static OPEN: AtomicUsize = AtomicUsize::new(0);

/// Counts an open connection until dropped.
struct Slot;
impl Slot {
    fn take() -> Option<Slot> {
        let mut n = OPEN.load(Ordering::Acquire);
        loop {
            if n >= MAX_CONNECTIONS {
                return None;
            }
            match OPEN.compare_exchange_weak(n, n + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(Slot),
                Err(now) => n = now,
            }
        }
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        OPEN.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The pages allowed to read the video back (the frame grabber draws it to a canvas).
pub(crate) fn allowed_origin(o: &str) -> bool {
    matches!(o, "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost")
        || (cfg!(debug_assertions) && o == "http://localhost:1420")
}

/// Start the server in the background and return its base address
/// (`http://127.0.0.1:<port>/<secret>`), to which the player adds `/<id>`.
pub fn start(state: Arc<AppState>) -> std::io::Result<String> {
    // No secret, no server: never fall back to something guessable.
    let secret = make_secret().map_err(|e| std::io::Error::other(e.to_string()))?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let base = format!("http://127.0.0.1:{port}/{secret}");
    std::thread::Builder::new().name("video-server".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            // Too many at once (a stuck or hostile client): close the newcomer.
            let Some(slot) = Slot::take() else { continue };
            let state = state.clone();
            let secret = secret.clone();
            let _ = std::thread::Builder::new().name("video-request".into()).spawn(move || {
                let _slot = slot;
                handle(stream, &state, &secret)
            });
        }
    })?;
    Ok(base)
}

/// 128 random bits as hex, from the system's random source.
fn make_secret() -> anyhow::Result<String> {
    crate::secure::random_hex(16)
}

/// A request, as far as the player needs.
#[derive(Debug, PartialEq)]
struct Request {
    /// GET or HEAD; anything else is answered with 405 and the connection closed.
    method_ok: bool,
    head: bool,
    id: Option<i64>,
    range: Option<String>,
    origin: Option<String>,
    /// The request said it carries a body, which a video player never sends.
    has_body: bool,
}

/// Read one request's head. `None` when the connection closed or the request is malformed.
fn read_request(r: &mut impl BufRead, secret: &str) -> Option<Request> {
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    let (mut range, mut origin, mut has_body) = (None, None, false);
    let mut total = line.len();
    loop {
        let mut h = String::new();
        let n = r.read_line(&mut h).ok()?;
        total += n;
        if n == 0 || total > 16 * 1024 {
            return None;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim(), v.trim());
            if k.eq_ignore_ascii_case("range") {
                range = Some(v.to_owned());
            } else if k.eq_ignore_ascii_case("origin") {
                origin = Some(v.to_owned());
            } else if (k.eq_ignore_ascii_case("content-length") && v != "0") || k.eq_ignore_ascii_case("transfer-encoding") {
                has_body = true;
            }
        }
    }
    let path = target.split(['?', '#']).next().unwrap_or("");
    let mut segs = path.trim_start_matches('/').splitn(2, '/');
    let id = match (segs.next(), segs.next()) {
        (Some(s), Some(id)) if crate::secure::constant_time_eq(s.as_bytes(), secret.as_bytes()) => id.trim_end_matches('/').parse().ok(),
        _ => None,
    };
    let head = method.eq_ignore_ascii_case("HEAD");
    Some(Request { method_ok: head || method.eq_ignore_ascii_case("GET"), head, id, range, origin, has_body })
}

fn handle(stream: TcpStream, state: &AppState, secret: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    // A client that stops reading can't hold the thread forever.
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let Ok(write_half) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let mut out = std::io::BufWriter::with_capacity(256 * 1024, write_half);
    // Keep the connection open for the player's next request.
    while let Some(req) = read_request(&mut reader, secret) {
        let cors = req.origin.as_deref().filter(|o| allowed_origin(o));
        if !req.method_ok || req.has_body {
            // Not something a player sends; answer and close rather than guess where the
            // next request starts.
            let _ = send_head(&mut out, "405 Method Not Allowed", "text/plain", 0, None, None).and_then(|_| out.flush());
            return;
        }
        let video = req.id.and_then(|id| {
            let conn = state.conn.lock();
            let p = db::get_photo(&conn, id).ok().flatten().filter(|p| metadata::is_video(&p.ext))?;
            // A video whose move is still waiting for its drive is really at its old place.
            let at = pending::source_of(&conn, id).ok().flatten().unwrap_or_else(|| p.path.clone());
            Some((at, p.ext))
        });
        let sent = match video {
            Some((path, ext)) => send_video(&mut out, std::path::Path::new(&path), &ext, req.range.as_deref(), req.head, cors),
            None => send_head(&mut out, "404 Not Found", "text/plain", 0, None, cors).and_then(|_| out.flush()),
        };
        if sent.is_err() {
            return;
        }
    }
}

fn send_head(w: &mut impl Write, status: &str, mime: &str, len: u64, range: Option<String>, cors: Option<&str>) -> std::io::Result<()> {
    write!(w, "HTTP/1.1 {status}\r\ncontent-type: {mime}\r\ncontent-length: {len}\r\naccept-ranges: bytes\r\n")?;
    if let Some(r) = range {
        write!(w, "content-range: {r}\r\n")?;
    }
    // The frame grabber for thumbnails reads the picture back, which needs CORS, but
    // only Lumio's own page is allowed to.
    if let Some(o) = cors {
        write!(w, "access-control-allow-origin: {o}\r\nvary: origin\r\n")?;
    }
    w.write_all(b"cache-control: no-store\r\n\r\n")
}

/// Send the whole video (200) or the asked-for part (206), streamed from disk so a long
/// film on a network drive is never read into memory whole.
fn send_video(w: &mut impl Write, path: &std::path::Path, ext: &str, range: Option<&str>, head_only: bool, cors: Option<&str>) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mime = metadata::video_mime(ext);
    let Ok(mut f) = std::fs::File::open(path) else {
        send_head(w, "404 Not Found", "text/plain", 0, None, cors)?;
        return w.flush();
    };
    let len = f.metadata()?.len();
    let (status, start, end, content_range) = match range {
        None => ("200 OK", 0, len.saturating_sub(1), None),
        Some(h) => match crate::parse_range(h, len) {
            Some((a, b)) => ("206 Partial Content", a, b, Some(format!("bytes {a}-{b}/{len}"))),
            None => {
                send_head(w, "416 Range Not Satisfiable", mime, 0, Some(format!("bytes */{len}")), cors)?;
                return w.flush();
            }
        },
    };
    let count = if len == 0 { 0 } else { end - start + 1 };
    send_head(w, status, mime, count, content_range, cors)?;
    if !head_only && count > 0 {
        f.seek(SeekFrom::Start(start))?;
        let copied = std::io::copy(&mut f.take(count), w)?;
        if copied < count {
            // The file shrank (or the drive went away) while sending; drop the connection
            // so the player doesn't wait for bytes that will never come.
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
    }
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Option<Request> {
        read_request(&mut std::io::Cursor::new(raw.as_bytes()), "s3cret")
    }

    #[test]
    fn reads_id_and_range() {
        let r = parse("GET /s3cret/42?m=1-1 HTTP/1.1\r\nHost: x\r\nRange: bytes=0-\r\nOrigin: tauri://localhost\r\n\r\n").unwrap();
        assert_eq!(
            r,
            Request { method_ok: true, head: false, id: Some(42), range: Some("bytes=0-".into()), origin: Some("tauri://localhost".into()), has_body: false }
        );
    }

    #[test]
    fn refuses_without_the_secret() {
        assert_eq!(parse("GET /wrong/42 HTTP/1.1\r\n\r\n").unwrap().id, None);
        assert_eq!(parse("GET /42 HTTP/1.1\r\n\r\n").unwrap().id, None);
        assert_eq!(parse("GET /s3cret/../etc HTTP/1.1\r\n\r\n").unwrap().id, None);
    }

    #[test]
    fn head_and_closed_connections() {
        assert!(parse("HEAD /s3cret/7 HTTP/1.1\r\n\r\n").unwrap().head);
        assert_eq!(parse(""), None);
        assert_eq!(parse("GET /s3cret/7 HTTP/1.1\r\nHost: x\r\n"), None);
    }

    #[test]
    fn only_get_and_head_without_a_body() {
        for m in ["POST", "PUT", "DELETE", "OPTIONS"] {
            assert!(!parse(&format!("{m} /s3cret/7 HTTP/1.1\r\n\r\n")).unwrap().method_ok, "{m}");
        }
        assert!(parse("GET /s3cret/7 HTTP/1.1\r\nContent-Length: 5\r\n\r\n").unwrap().has_body);
        assert!(parse("GET /s3cret/7 HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap().has_body);
        assert!(!parse("GET /s3cret/7 HTTP/1.1\r\nContent-Length: 0\r\n\r\n").unwrap().has_body);
    }

    #[test]
    fn cors_only_for_lumio() {
        assert!(allowed_origin("tauri://localhost") && allowed_origin("http://tauri.localhost"));
        assert!(!allowed_origin("https://evil.example") && !allowed_origin("null") && !allowed_origin("*"));
        let mut out = Vec::new();
        send_head(&mut out, "200 OK", "video/mp4", 0, None, None).unwrap();
        assert!(!String::from_utf8(out).unwrap().contains("access-control-allow-origin"));
    }

    #[test]
    fn connections_are_capped() {
        let held: Vec<Slot> = std::iter::from_fn(Slot::take).take(MAX_CONNECTIONS + 5).collect();
        assert!(held.len() <= MAX_CONNECTIONS);
        assert!(Slot::take().is_none());
        drop(held);
        assert!(Slot::take().is_some());
    }

    #[test]
    fn secrets_differ() {
        let (a, b) = (make_secret().unwrap(), make_secret().unwrap());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }

    fn send(range: Option<&str>) -> String {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("clip.mp4");
        std::fs::write(&p, b"0123456789").unwrap();
        let mut out = Vec::new();
        send_video(&mut out, &p, "mp4", range, false, Some("tauri://localhost")).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn whole_file_without_range() {
        let s = send(None);
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"), "{s}");
        assert!(s.contains("content-length: 10\r\n") && s.ends_with("\r\n\r\n0123456789"));
    }

    #[test]
    fn parts_with_range() {
        let s = send(Some("bytes=3-"));
        assert!(s.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert!(s.contains("content-range: bytes 3-9/10\r\n") && s.ends_with("\r\n\r\n3456789"));
        let s = send(Some("bytes=2-4"));
        assert!(s.contains("content-length: 3\r\n") && s.ends_with("234"));
        assert!(send(Some("bytes=20-")).starts_with("HTTP/1.1 416"));
    }
}
