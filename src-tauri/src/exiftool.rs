//! A small wrapper around ExifTool running in `-stay_open` mode.
//!
//! ExifTool (https://exiftool.org, Phil Harvey, Artistic/GPL licence) is the most
//! complete open-source metadata reader/writer available. Lumio bundles a copy
//! (see `scripts/fetch-exiftool.mjs`) and falls back to an `exiftool` found on PATH.

use anyhow::{anyhow, bail, Context, Result};
use parking_lot::Mutex;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// How long one ExifTool command may take before the process is restarted. Reading a batch
/// of files from a slow network drive gets more time (see `read_json`).
const TIMEOUT: Duration = Duration::from_secs(60);

struct Proc {
    child: Child,
    stdin: ChildStdin,
    /// Lines of ExifTool's output, read by a helper thread so a hung ExifTool can time out.
    lines: Receiver<Vec<u8>>,
    /// The last of what ExifTool wrote to stderr, for the log when a command fails.
    stderr: Arc<Mutex<String>>,
}

pub struct ExifTool {
    program: Option<(PathBuf, Vec<String>)>,
    proc: Mutex<Option<Proc>>,
    seq: AtomicU64,
    /// Where `%`-free stand-ins and extracted previews go.
    scratch: PathBuf,
}

/// One argument as a line of ExifTool's `-@ -` argument stream. Lines are separated by
/// newlines and a line starting with `#` is a comment, so anything with a line break or a
/// leading `#` is sent as a `#[CSTR]` line with C escapes. NUL can't be sent at all.
pub fn encode_arg(a: &str) -> Result<String> {
    if a.contains('\0') {
        bail!("text contains a NUL character");
    }
    if a.contains('\n') || a.contains('\r') || a.starts_with('#') {
        let esc = a.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r");
        // A leading '#' is fine after the #[CSTR] prefix.
        Ok(format!("#[CSTR]{esc}"))
    } else {
        Ok(a.to_string())
    }
}

/// A file path as an ExifTool argument: absolute, and without characters the argument
/// stream can't carry safely. Being absolute is what stops a file name from being read as
/// an option (it starts with `/` or a drive letter, never `-`). ExifTool's `--` can't be
/// used for that in `-stay_open` mode: it turns the following `-execute` into a file name.
pub fn path_arg(p: &Path) -> Result<String> {
    let s = p.to_string_lossy().into_owned();
    if !p.is_absolute() {
        bail!("not an absolute path: {s}");
    }
    if s.contains(['\n', '\r', '\0']) {
        bail!("file names with line breaks are not supported: {s:?}");
    }
    Ok(s)
}

/// A path for an option where ExifTool expands `%d`, `%f`, `%t`… (`-o`, `-W`,
/// `-TagsFromFile`). ExifTool has no reliable escape for a literal `%` there ("%%d" still
/// expands), so such paths are refused; callers go through a `%`-free stand-in instead
/// (see `ExifTool::plain`).
pub fn fmt_path_arg(p: &Path) -> Result<String> {
    let s = path_arg(p)?;
    if s.contains('%') {
        bail!("internal: '%' in a format path: {s}");
    }
    Ok(s)
}

/// A `%`-free stand-in for a file: the file itself when its path has no `%`, otherwise a
/// link to it (or, failing that, a copy) in Lumio's scratch folder, removed on drop.
pub struct Plain {
    path: PathBuf,
    temp: bool,
}

impl Plain {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Plain {
    fn drop(&mut self) {
        if self.temp {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl ExifTool {
    /// `resource_dir` is the Tauri resource directory, which may contain a bundled
    /// `exiftool/` folder.
    pub fn new(resource_dir: Option<PathBuf>) -> Self {
        Self {
            program: locate(resource_dir),
            proc: Mutex::new(None),
            seq: AtomicU64::new(1),
            scratch: std::env::temp_dir(),
        }
    }

    /// Use `dir` (Lumio's own cache folder) for temporary files.
    pub fn with_scratch(mut self, dir: PathBuf) -> Self {
        self.scratch = dir;
        self
    }

    /// A fresh, unguessable name in the scratch folder.
    fn scratch_path(&self, ext: &str) -> Result<PathBuf> {
        let p = self.scratch.join(format!("et-{}{ext}", crate::secure::random_hex(12)?));
        if p.to_string_lossy().contains('%') {
            bail!("Lumio's temporary folder has a '%' in its path ({})", self.scratch.display());
        }
        Ok(p)
    }

    /// See `Plain`.
    pub fn plain(&self, p: &Path) -> Result<Plain> {
        if !p.to_string_lossy().contains('%') {
            return Ok(Plain { path: p.to_path_buf(), temp: false });
        }
        let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let alias = self.scratch_path(&ext)?;
        std::fs::create_dir_all(&self.scratch)?;
        if std::fs::hard_link(p, &alias).is_err() {
            std::fs::copy(p, &alias).with_context(|| format!("copying {}", p.display()))?;
        }
        Ok(Plain { path: alias, temp: true })
    }

    pub fn available(&self) -> bool {
        self.program.is_some()
    }

    pub fn describe(&self) -> String {
        match &self.program {
            Some((p, a)) => format!("{} {}", p.display(), a.join(" ")),
            None => "not found".into(),
        }
    }

    fn spawn(&self) -> Result<Proc> {
        let (prog, pre) = self
            .program
            .as_ref()
            .ok_or_else(|| anyhow!("ExifTool was not found. Metadata reading and writing need ExifTool."))?;
        let mut cmd = Command::new(prog);
        cmd.args(pre)
            .args(["-stay_open", "True", "-@", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().with_context(|| format!("starting {}", prog.display()))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let stderr_pipe = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("exiftool-out".into()).spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match r.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        })?;
        let stderr = Arc::new(Mutex::new(String::new()));
        let sink = stderr.clone();
        std::thread::Builder::new().name("exiftool-err".into()).spawn(move || {
            let mut r = BufReader::new(stderr_pipe);
            let mut line = String::new();
            while r.read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
                let mut s = sink.lock();
                s.push_str(&line);
                if s.len() > 8192 {
                    let cut = s.len() - 4096;
                    let cut = (cut..s.len()).find(|i| s.is_char_boundary(*i)).unwrap_or(s.len());
                    s.drain(..cut);
                }
                line.clear();
            }
        })?;
        Ok(Proc { child, stdin, lines, stderr })
    }

    /// Run one ExifTool command (one argument per element) and return its stdout.
    pub fn run(&self, args: &[String]) -> Result<Vec<u8>> {
        self.run_for(args, TIMEOUT)
    }

    /// `run` with a time limit; a command that takes longer kills and restarts ExifTool.
    pub fn run_for(&self, args: &[String], timeout: Duration) -> Result<Vec<u8>> {
        let mut guard = self.proc.lock();
        let mut last = anyhow!("ExifTool did not run");
        for _attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(self.spawn()?);
            }
            let Some(p) = guard.as_mut() else { continue };
            match Self::exec(p, args, self.seq.fetch_add(1, Ordering::Relaxed), timeout) {
                Ok(out) => return Ok(out),
                Err(e) => {
                    // The process may have died or hung; restart it (once).
                    if let Some(mut dead) = guard.take() {
                        let _ = dead.child.kill();
                        let _ = dead.child.wait();
                        let err = dead.stderr.lock().clone();
                        if !err.trim().is_empty() {
                            log::warn!("ExifTool stderr: {}", err.trim());
                        }
                    }
                    // A timeout or bad argument would only happen again.
                    let retry = !(e.to_string().contains("timed out") || e.to_string().contains("NUL"));
                    last = e;
                    if !retry {
                        break;
                    }
                }
            }
        }
        Err(last)
    }

    fn exec(p: &mut Proc, args: &[String], seq: u64, timeout: Duration) -> Result<Vec<u8>> {
        // Leftovers from an earlier command that failed half way must not be read as ours.
        while p.lines.try_recv().is_ok() {}
        let mut buf = String::from("-charset\nfilename=utf8\n");
        for a in args {
            buf.push_str(&encode_arg(a)?);
            buf.push('\n');
        }
        buf.push_str(&format!("-execute{seq}\n"));
        p.stdin.write_all(buf.as_bytes())?;
        p.stdin.flush()?;
        let marker = format!("{{ready{seq}}}");
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match p.lines.recv_timeout(left) {
                Ok(l) => l,
                Err(RecvTimeoutError::Timeout) => bail!("ExifTool timed out after {}s", timeout.as_secs()),
                Err(RecvTimeoutError::Disconnected) => bail!("ExifTool exited unexpectedly"),
            };
            if String::from_utf8_lossy(&line).trim_end() == marker {
                break;
            }
            out.extend_from_slice(&line);
        }
        Ok(out)
    }

    /// Read metadata for many files at once. Returns one JSON object per file.
    pub fn read_json(&self, files: &[PathBuf]) -> Result<Vec<Value>> {
        if files.is_empty() {
            return Ok(vec![]);
        }
        let mut args: Vec<String> = ["-j", "-n", "-fast", "-api", "largefilesupport=1", "-api", "QuickTimeUTC=1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        for t in READ_TAGS {
            args.push(format!("-{t}"));
        }
        for f in files {
            args.push(path_arg(f)?);
        }
        let out = self.run_for(&args, TIMEOUT + Duration::from_secs(5 * files.len() as u64))?;
        if out.iter().all(|b| b.is_ascii_whitespace()) {
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_slice(&out).context("parsing ExifTool JSON")?;
        Ok(v.as_array().cloned().unwrap_or_default())
    }

    /// Write tags. `assignments` are ExifTool arguments such as `-XMP:Rating=3`.
    pub fn write(&self, file: &Path, assignments: &[String]) -> Result<()> {
        let mut args: Vec<String> = vec!["-overwrite_original".into(), "-codedcharacterset=utf8".into(), "-m".into()];
        args.extend(assignments.iter().cloned());
        args.push(path_arg(file)?);
        let out = self.run(&args)?;
        let text = String::from_utf8_lossy(&out);
        if !(text.contains("1 image files updated") || text.contains("unchanged") || text.contains("created")) {
            bail!("ExifTool could not write {}: {}", file.display(), text.trim());
        }
        Ok(())
    }

    /// Create an XMP sidecar for `file` at `xmp` (copying existing XMP/EXIF data).
    pub fn create_sidecar(&self, file: &Path, xmp: &Path) -> Result<()> {
        // `-o` expands '%' codes, so a path with '%' is written in scratch and moved.
        let direct = !xmp.to_string_lossy().contains('%');
        let out = if direct { xmp.to_path_buf() } else { self.scratch_path(".xmp")? };
        if !direct {
            std::fs::create_dir_all(&self.scratch)?;
        }
        let args = vec!["-o".to_string(), fmt_path_arg(&out)?, path_arg(file)?];
        self.run(&args)?;
        if !direct && out.exists() && std::fs::rename(&out, xmp).is_err() {
            std::fs::copy(&out, xmp)?;
            let _ = std::fs::remove_file(&out);
        }
        if !xmp.exists() {
            bail!("could not create sidecar {}", xmp.display());
        }
        Ok(())
    }

    /// Extract the largest embedded JPEG preview (RAW files, some TIFF/HEIC).
    pub fn extract_preview(&self, file: &Path, tmp_dir: &Path) -> Result<Option<Vec<u8>>> {
        let id = self.seq.fetch_add(1, Ordering::Relaxed);
        let base = if tmp_dir.to_string_lossy().contains('%') { self.scratch.as_path() } else { tmp_dir };
        let dir = base.join(format!("prev{id}-{}", crate::secure::random_hex(6)?));
        std::fs::create_dir_all(&dir)?;
        // The "%t" we add is the only format code (fmt_path_arg refuses any other '%').
        let pattern = format!("{}{}%t.jpg", fmt_path_arg(&dir)?, std::path::MAIN_SEPARATOR);
        let args = vec![
            "-b".to_string(),
            "-JpgFromRaw".into(),
            "-PreviewImage".into(),
            "-OtherImage".into(),
            "-ThumbnailImage".into(),
            "-W".into(),
            pattern,
            path_arg(file)?,
        ];
        let res = self.run(&args);
        let mut best: Option<Vec<u8>> = None;
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if let Ok(mut f) = std::fs::File::open(e.path()) {
                    let mut data = Vec::new();
                    if f.read_to_end(&mut data).is_ok()
                        && data.len() > 2
                        && data[0] == 0xFF
                        && data[1] == 0xD8
                        && best.as_ref().is_none_or(|b| data.len() > b.len())
                    {
                        best = Some(data);
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        res?;
        Ok(best)
    }
}

impl Drop for ExifTool {
    fn drop(&mut self) {
        if let Some(mut p) = self.proc.lock().take() {
            let _ = p.stdin.write_all(b"-stay_open\nFalse\n");
            let _ = p.stdin.flush();
            let _ = p.child.wait();
        }
    }
}

pub const READ_TAGS: &[&str] = &[
    "ImageWidth",
    "ImageHeight",
    "ExifImageWidth",
    "ExifImageHeight",
    "Orientation",
    "DateTimeOriginal",
    "CreateDate",
    "DateCreated",
    "ModifyDate",
    "Rating",
    "Label",
    "XMP-dc:Title",
    "ObjectName",
    "XMP-dc:Description",
    "Caption-Abstract",
    "ImageDescription",
    "Subject",
    "Keywords",
    "HierarchicalSubject",
    "XMP-dc:Creator",
    "Artist",
    "By-line",
    "XMP-dc:Rights",
    "Copyright",
    "CopyrightNotice",
    "GPSLatitude",
    "GPSLongitude",
    "GPSAltitude",
    "City",
    "State",
    "Country",
    "Location",
    "Sub-location",
    "Province-State",
    "Country-PrimaryLocationName",
    "Make",
    "Model",
    "LensModel",
    "Lens",
    "LensID",
    "ISO",
    "FNumber",
    "ExposureTime",
    "FocalLength",
    "Duration",
    "Rotation",
];

fn locate(resource_dir: Option<PathBuf>) -> Option<(PathBuf, Vec<String>)> {
    if let Some(res) = resource_dir {
        let base = res.join("exiftool");
        #[cfg(windows)]
        {
            let exe = base.join("exiftool.exe");
            if exe.exists() {
                return Some((exe, vec![]));
            }
        }
        #[cfg(not(windows))]
        {
            let script = base.join("exiftool");
            if script.exists() {
                // An absolute perl, so a "perl" earlier on PATH can't stand in for it.
                if let Some(perl) = ["/usr/bin/perl", "/usr/local/bin/perl", "/bin/perl", "/opt/homebrew/bin/perl"]
                    .iter()
                    .map(PathBuf::from)
                    .find(|p| p.is_file())
                {
                    return Some((perl, vec![script.to_string_lossy().into_owned()]));
                }
            }
        }
    }
    // Fall back to an ExifTool on PATH (and common Homebrew locations on macOS).
    let candidates: &[&str] = if cfg!(windows) {
        &["exiftool.exe", "exiftool"]
    } else {
        &["exiftool", "/opt/homebrew/bin/exiftool", "/usr/local/bin/exiftool", "/usr/bin/exiftool"]
    };
    for c in candidates {
        let mut cmd = Command::new(c);
        cmd.arg("-ver").stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        if cmd.status().map(|s| s.success()).unwrap_or(false) {
            return Some((PathBuf::from(c), vec![]));
        }
    }
    None
}

/// The bundled ExifTool (or one on PATH) for tests. `None` skips a test locally when
/// there is none; CI sets LUMIO_REQUIRE_EXIFTOOL so a missing ExifTool fails instead.
#[cfg(test)]
pub fn test_tool() -> Option<ExifTool> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    let et = ExifTool::new(Some(dir)).with_scratch(tempfile::tempdir().unwrap().keep());
    if !et.available() {
        assert!(
            std::env::var_os("LUMIO_REQUIRE_EXIFTOOL").is_none(),
            "ExifTool is required (run npm run fetch-exiftool)"
        );
        eprintln!("ExifTool not installed; skipping");
        return None;
    }
    Some(et)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_arguments_pass_through() {
        assert_eq!(encode_arg("-XMP-dc:Title=Sunset").unwrap(), "-XMP-dc:Title=Sunset");
        assert_eq!(encode_arg("-XMP-dc:Title=50% off").unwrap(), "-XMP-dc:Title=50% off");
    }

    #[test]
    fn line_breaks_and_comments_are_escaped() {
        assert_eq!(encode_arg("a\nb").unwrap(), "#[CSTR]a\\nb");
        // A bare carriage return used to go through raw and split the argument.
        assert_eq!(encode_arg("a\rb").unwrap(), "#[CSTR]a\\rb");
        assert_eq!(encode_arg("a\\b\r\n").unwrap(), "#[CSTR]a\\\\b\\r\\n");
        // A leading '#' would make the line a comment (or a #[...] directive).
        assert_eq!(encode_arg("#[CSTR]x").unwrap(), "#[CSTR]#[CSTR]x");
        assert_eq!(encode_arg("# note").unwrap(), "#[CSTR]# note");
        for a in ["a\nb", "x\ry", "#c"] {
            assert!(!encode_arg(a).unwrap().contains(['\n', '\r']));
        }
    }

    #[test]
    fn nul_is_refused() {
        assert!(encode_arg("a\0b").is_err());
    }

    #[test]
    fn paths_must_be_absolute_and_single_line() {
        let abs = if cfg!(windows) { "C:\\p\\a.jpg" } else { "/p/a.jpg" };
        assert!(path_arg(Path::new(abs)).is_ok());
        assert!(path_arg(Path::new("relative.jpg")).is_err());
        assert!(path_arg(Path::new("-relative.jpg")).is_err());
        let bad = format!("{abs}\n-delete_original!");
        assert!(path_arg(Path::new(&bad)).is_err());
        assert!(path_arg(Path::new(&format!("{abs}\r"))).is_err());
    }

    #[test]
    fn percent_signs_never_reach_format_options() {
        let p = if cfg!(windows) { "C:\\50% sale\\%d.jpg" } else { "/50% sale/%d.jpg" };
        assert!(fmt_path_arg(Path::new(p)).is_err());
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("50% sale.jpg");
        std::fs::write(&f, b"x").unwrap();
        let et = ExifTool::new(None).with_scratch(dir.path().join("scratch"));
        let alias = et.plain(&f).unwrap();
        assert!(!alias.path().to_string_lossy().contains('%'));
        assert_eq!(std::fs::read(alias.path()).unwrap(), b"x");
        let kept = alias.path().to_path_buf();
        drop(alias);
        assert!(!kept.exists() && f.exists());
        let plain = dir.path().join("plain.jpg");
        assert_eq!(et.plain(&plain).unwrap().path(), plain.as_path());
    }

    #[test]
    fn awkward_values_and_paths_round_trip_through_exiftool() {
        let Some(et) = test_tool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("50% sale #1");
        std::fs::create_dir_all(&sub).unwrap();
        let f = sub.join("-img %d.jpg");
        let img = image::RgbImage::new(8, 8);
        img.save(&f).unwrap();
        let title = "line one\r\nline two\rthree #notacomment";
        et.write(&f, &[format!("-XMP-dc:Title={title}"), "-XMP-dc:Description=#hash first".into()]).unwrap();
        let v = et.read_json(std::slice::from_ref(&f)).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0]["Description"].as_str(), Some("#hash first"));
        let got = v[0]["Title"].as_str().unwrap_or_default().replace("\r\n", "\n");
        assert!(got.contains("line one") && got.contains("three #notacomment"), "{got:?}");
        // The sidecar lands at the literal path, not one with %d expanded.
        let xmp = sub.join("-img %d.xmp");
        et.create_sidecar(&f, &xmp).unwrap();
        assert!(xmp.exists());
        assert_eq!(std::fs::read_dir(&sub).unwrap().count(), 2, "nothing else may be created");
        assert!(et.write(&f, &["-XMP-dc:Title=a\0b".into()]).is_err());
    }
}
