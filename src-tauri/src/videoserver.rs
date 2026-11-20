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
use crate::{db, metadata};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// Start the server in the background and return its base address
/// (`http://127.0.0.1:<port>/<secret>`), to which the player adds `/<id>`.
pub fn start(state: Arc<AppState>) -> std::io::Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let secret = make_secret();
    let base = format!("http://127.0.0.1:{port}/{secret}");
    std::thread::Builder::new().name("video-server".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            let secret = secret.clone();
            let _ = std::thread::Builder::new()
                .name("video-request".into())
                .spawn(move || handle(stream, &state, &secret));
        }
    })?;
    Ok(base)
}

/// 128 random bits as hex, from the system's random source.
fn make_secret() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    let from_os = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_ok();
    if !from_os {
        // Not expected on Linux; fall back to the standard library's random hash keys.
        use std::hash::{BuildHasher, Hasher};
        for chunk in b.chunks_mut(8) {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
            chunk.copy_from_slice(&h.finish().to_le_bytes()[..chunk.len()]);
        }
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A request, as far as the player needs: the method, the photo id and the Range header.
#[derive(Debug, PartialEq)]
struct Request {
    head: bool,
    id: Option<i64>,
    range: Option<String>,
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
    let mut range = None;
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
            if k.trim().eq_ignore_ascii_case("range") {
                range = Some(v.trim().to_owned());
            }
        }
    }
    let path = target.split(['?', '#']).next().unwrap_or("");
    let mut segs = path.trim_start_matches('/').splitn(2, '/');
    let id = match (segs.next(), segs.next()) {
        (Some(s), Some(id)) if s == secret => id.trim_end_matches('/').parse().ok(),
        _ => None,
    };
    Some(Request { head: method.eq_ignore_ascii_case("HEAD"), id, range })
}

fn handle(stream: TcpStream, state: &AppState, secret: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Ok(write_half) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let mut out = std::io::BufWriter::with_capacity(256 * 1024, write_half);
    // Keep the connection open for the player's next request.
    while let Some(req) = read_request(&mut reader, secret) {
        let video = req.id.and_then(|id| db::get_photo(&state.conn.lock(), id).ok().flatten()).filter(|p| metadata::is_video(&p.ext));
        let sent = match video {
            Some(p) => send_video(&mut out, std::path::Path::new(&p.path), &p.ext, req.range.as_deref(), req.head),
            None => send_head(&mut out, "404 Not Found", "text/plain", 0, None).and_then(|_| out.flush()),
        };
        if sent.is_err() {
            return;
        }
    }
}

fn send_head(w: &mut impl Write, status: &str, mime: &str, len: u64, range: Option<String>) -> std::io::Result<()> {
    write!(w, "HTTP/1.1 {status}\r\ncontent-type: {mime}\r\ncontent-length: {len}\r\naccept-ranges: bytes\r\n")?;
    if let Some(r) = range {
        write!(w, "content-range: {r}\r\n")?;
    }
    // The frame grabber for thumbnails reads the picture back, which needs this.
    w.write_all(b"access-control-allow-origin: *\r\ncache-control: no-store\r\n\r\n")
}

/// Send the whole video (200) or the asked-for part (206), streamed from disk so a long
/// film on a network drive is never read into memory whole.
fn send_video(w: &mut impl Write, path: &std::path::Path, ext: &str, range: Option<&str>, head_only: bool) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mime = metadata::video_mime(ext);
    let Ok(mut f) = std::fs::File::open(path) else {
        send_head(w, "404 Not Found", "text/plain", 0, None)?;
        return w.flush();
    };
    let len = f.metadata()?.len();
    let (status, start, end, content_range) = match range {
        None => ("200 OK", 0, len.saturating_sub(1), None),
        Some(h) => match crate::parse_range(h, len) {
            Some((a, b)) => ("206 Partial Content", a, b, Some(format!("bytes {a}-{b}/{len}"))),
            None => {
                send_head(w, "416 Range Not Satisfiable", mime, 0, Some(format!("bytes */{len}")))?;
                return w.flush();
            }
        },
    };
    let count = if len == 0 { 0 } else { end - start + 1 };
    send_head(w, status, mime, count, content_range)?;
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
        let r = parse("GET /s3cret/42?m=1-1 HTTP/1.1\r\nHost: x\r\nRange: bytes=0-\r\n\r\n").unwrap();
        assert_eq!(r, Request { head: false, id: Some(42), range: Some("bytes=0-".into()) });
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
    fn secrets_differ() {
        let (a, b) = (make_secret(), make_secret());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }

    fn send(range: Option<&str>) -> String {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("lumio-vs-test-{}-{n}.mp4", std::process::id()));
        std::fs::write(&p, b"0123456789").unwrap();
        let mut out = Vec::new();
        send_video(&mut out, &p, "mp4", range, false).unwrap();
        let _ = std::fs::remove_file(&p);
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
