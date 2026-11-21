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
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

struct Proc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct ExifTool {
    program: Option<(PathBuf, Vec<String>)>,
    proc: Mutex<Option<Proc>>,
    seq: AtomicU64,
}

impl ExifTool {
    /// `resource_dir` is the Tauri resource directory, which may contain a bundled
    /// `exiftool/` folder.
    pub fn new(resource_dir: Option<PathBuf>) -> Self {
        Self { program: locate(resource_dir), proc: Mutex::new(None), seq: AtomicU64::new(1) }
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
        let (prog, pre) = self.program.as_ref().ok_or_else(|| {
            anyhow!("ExifTool was not found. Metadata reading and writing need ExifTool.")
        })?;
        let mut cmd = Command::new(prog);
        cmd.args(pre)
            .args(["-stay_open", "True", "-@", "-", "-common_args", "-charset", "filename=utf8"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().with_context(|| format!("starting {}", prog.display()))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Proc { child, stdin, stdout })
    }

    /// Run one ExifTool command (one argument per element) and return its stdout.
    pub fn run(&self, args: &[String]) -> Result<Vec<u8>> {
        let mut guard = self.proc.lock();
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(self.spawn()?);
            }
            let p = guard.as_mut().unwrap();
            match Self::exec(p, args, self.seq.fetch_add(1, Ordering::Relaxed)) {
                Ok(out) => return Ok(out),
                Err(e) => {
                    // The process may have died; restart once.
                    if let Some(mut dead) = guard.take() {
                        let _ = dead.child.kill();
                    }
                    if attempt == 1 {
                        return Err(e);
                    }
                }
            }
        }
        unreachable!()
    }

    fn exec(p: &mut Proc, args: &[String], seq: u64) -> Result<Vec<u8>> {
        let mut buf = String::new();
        for a in args {
            if a.contains('\n') {
                // Arguments are newline-separated; ExifTool understands "#[CSTR]" lines
                // with C escapes, which lets us pass multi-line captions.
                buf.push_str("#[CSTR]");
                buf.push_str(&a.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', ""));
            } else {
                buf.push_str(a);
            }
            buf.push('\n');
        }
        buf.push_str(&format!("-execute{seq}\n"));
        p.stdin.write_all(buf.as_bytes())?;
        p.stdin.flush()?;
        let marker = format!("{{ready{seq}}}");
        let mut out = Vec::new();
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = p.stdout.read_until(b'\n', &mut line)?;
            if n == 0 {
                bail!("ExifTool exited unexpectedly");
            }
            let trimmed = String::from_utf8_lossy(&line);
            if trimmed.trim_end() == marker {
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
            args.push(f.to_string_lossy().into_owned());
        }
        let out = self.run(&args)?;
        if out.iter().all(|b| b.is_ascii_whitespace()) {
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_slice(&out).context("parsing ExifTool JSON")?;
        Ok(v.as_array().cloned().unwrap_or_default())
    }

    /// Write tags. `assignments` are ExifTool arguments such as `-XMP:Rating=3`.
    pub fn write(&self, file: &Path, assignments: &[String]) -> Result<()> {
        let mut args: Vec<String> =
            vec!["-overwrite_original".into(), "-codedcharacterset=utf8".into(), "-m".into()];
        args.extend(assignments.iter().cloned());
        args.push(file.to_string_lossy().into_owned());
        let out = self.run(&args)?;
        let text = String::from_utf8_lossy(&out);
        if !(text.contains("1 image files updated") || text.contains("unchanged") || text.contains("created")) {
            bail!("ExifTool could not write {}: {}", file.display(), text.trim());
        }
        Ok(())
    }

    /// Create an XMP sidecar for `file` at `xmp` (copying existing XMP/EXIF data).
    pub fn create_sidecar(&self, file: &Path, xmp: &Path) -> Result<()> {
        let args = vec![
            "-o".to_string(),
            xmp.to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
        ];
        self.run(&args)?;
        if !xmp.exists() {
            bail!("could not create sidecar {}", xmp.display());
        }
        Ok(())
    }

    /// Extract the largest embedded JPEG preview (RAW files, some TIFF/HEIC).
    pub fn extract_preview(&self, file: &Path, tmp_dir: &Path) -> Result<Option<Vec<u8>>> {
        let id = self.seq.fetch_add(1, Ordering::Relaxed);
        let dir = tmp_dir.join(format!("prev{id}"));
        std::fs::create_dir_all(&dir)?;
        let pattern = dir.join("%t.jpg");
        let args = vec![
            "-b".to_string(),
            "-JpgFromRaw".into(),
            "-PreviewImage".into(),
            "-OtherImage".into(),
            "-ThumbnailImage".into(),
            "-W".into(),
            pattern.to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
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
                        && best.as_ref().map_or(true, |b| data.len() > b.len())
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
    "ImageWidth", "ImageHeight", "ExifImageWidth", "ExifImageHeight", "Orientation",
    "DateTimeOriginal", "CreateDate", "DateCreated", "ModifyDate",
    "Rating", "Label",
    "XMP-dc:Title", "ObjectName",
    "XMP-dc:Description", "Caption-Abstract", "ImageDescription",
    "Subject", "Keywords", "HierarchicalSubject",
    "XMP-dc:Creator", "Artist", "By-line",
    "XMP-dc:Rights", "Copyright", "CopyrightNotice",
    "GPSLatitude", "GPSLongitude", "GPSAltitude",
    "City", "State", "Country", "Location", "Sub-location", "Province-State", "Country-PrimaryLocationName",
    "Make", "Model", "LensModel", "Lens", "LensID", "ISO", "FNumber", "ExposureTime", "FocalLength",
    "Duration", "Rotation",
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
                return Some((PathBuf::from("perl"), vec![script.to_string_lossy().into_owned()]));
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
