// Downloads ExifTool (https://exiftool.org, by Phil Harvey) into src-tauri/resources/exiftool
// so it is bundled with the app. Run automatically before `tauri build`.
//
//   node scripts/fetch-exiftool.mjs            # for the current platform
//   node scripts/fetch-exiftool.mjs --windows  # force the Windows build
//
// macOS / Linux get the portable Perl version (Perl ships with both).
// Windows gets the stand-alone exiftool.exe build.
//
// The version and the SHA-256 of each archive are pinned below. A download whose bytes do
// not match is rejected before anything is unpacked, so a compromised mirror cannot slip
// a different program into the installers. To update: change VERSION, download both
// archives, compare their SHA-256 with https://exiftool.org/checksums.txt, paste them here.
import { mkdirSync, existsSync, rmSync, writeFileSync, readdirSync, cpSync, chmodSync, mkdtempSync, readFileSync } from "node:fs";
import { join, dirname, isAbsolute, normalize, sep } from "node:path";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

export const VERSION = "13.64";
export const SHA256 = {
  tar: "dcb9f6be1f6705ee826a92d1522e3fec1d0b3749cdc061b03c46873d5fe204fc", // Image-ExifTool-13.64.tar.gz
  zip: "72c29c44d1e860eebcaefa71692d979af9858fe9a6d7d791a5e7f1dac77804af", // exiftool-13.64_64.zip
};

/** sha256 of a buffer, lower-case hex. */
export function sha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

/** Throws unless every archive member stays inside the folder it is unpacked into. */
export function checkEntries(names) {
  for (const raw of names) {
    const n = raw.replace(/\\/g, "/");
    if (!n) continue;
    if (isAbsolute(n) || /^[A-Za-z]:/.test(n) || n.split("/").includes("..") || n.includes("\0")) {
      throw new Error(`Unsafe path in ExifTool archive: ${raw}`);
    }
  }
}

async function get(urls, expected) {
  for (const u of urls) {
    try {
      const r = await fetch(u, { redirect: "follow", headers: { "User-Agent": "frameloft-build" } });
      if (!r.ok) {
        console.warn(`  ${u}: HTTP ${r.status}`);
        continue;
      }
      const buf = Buffer.from(await r.arrayBuffer());
      const got = sha256(buf);
      if (got !== expected) {
        console.warn(`  ${u}: checksum mismatch (got ${got}, expected ${expected}), ignoring this download`);
        continue;
      }
      return buf;
    } catch (e) {
      console.warn(`  ${u}: ${e.message}`);
    }
  }
  throw new Error("Could not download a verified copy of ExifTool. Check your internet connection, or install it manually (see README).");
}

/** Lists the archive with tar, checks every member, then unpacks it. */
function unpack(archive, into, gz) {
  const list = execFileSync("tar", [gz ? "-tzf" : "-tf", archive], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  checkEntries(list.split(/\r?\n/));
  execFileSync("tar", [gz ? "-xzf" : "-xf", archive, "-C", into]);
  // Second line of defence: nothing may have landed outside `into` through a link.
  for (const f of readdirSync(into, { recursive: true })) {
    if (normalize(join(into, String(f))).indexOf(normalize(into) + sep) !== 0) throw new Error("Archive escaped its folder");
  }
}

async function main() {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const dest = join(root, "src-tauri", "resources", "exiftool");
  const windows = process.argv.includes("--windows") || process.platform === "win32";
  const marker = join(dest, windows ? "exiftool.exe" : "exiftool");
  const versionFile = join(dest, "VERSION");

  if (existsSync(marker) && existsSync(versionFile) && readFileSync(versionFile, "utf8").trim() === VERSION && !process.argv.includes("--force")) {
    console.log(`ExifTool ${VERSION} already present in ${dest}`);
    return;
  }

  console.log(`Fetching ExifTool ${VERSION} for ${windows ? "Windows" : "macOS/Linux"}…`);
  const work = mkdtempSync(join(tmpdir(), "frameloft-exiftool-"));
  try {
    rmSync(dest, { recursive: true, force: true });
    mkdirSync(dest, { recursive: true });
    writeFileSync(join(dest, ".keep"), "");
    if (windows) {
      const name = `exiftool-${VERSION}_64.zip`;
      const zip = await get([`https://exiftool.org/${name}`, `https://sourceforge.net/projects/exiftool/files/${name}/download`], SHA256.zip);
      const zipPath = join(work, name);
      writeFileSync(zipPath, zip);
      unpack(zipPath, work, false); // bsdtar on Windows 10+ reads zip
      const dir = readdirSync(work).map((d) => join(work, d)).find((d) => existsSync(join(d, "exiftool_files")));
      if (!dir) throw new Error("Unexpected ExifTool zip layout");
      const exe = readdirSync(dir).find((f) => f.toLowerCase().endsWith(".exe"));
      if (!exe) throw new Error("exiftool.exe missing from the zip");
      cpSync(join(dir, "exiftool_files"), join(dest, "exiftool_files"), { recursive: true });
      cpSync(join(dir, exe), join(dest, "exiftool.exe"));
    } else {
      const name = `Image-ExifTool-${VERSION}.tar.gz`;
      const tgz = await get([`https://exiftool.org/${name}`, `https://sourceforge.net/projects/exiftool/files/${name}/download`], SHA256.tar);
      const tgzPath = join(work, name);
      writeFileSync(tgzPath, tgz);
      unpack(tgzPath, work, true);
      const dir = join(work, `Image-ExifTool-${VERSION}`);
      cpSync(join(dir, "exiftool"), join(dest, "exiftool"));
      cpSync(join(dir, "lib"), join(dest, "lib"), { recursive: true });
      chmodSync(join(dest, "exiftool"), 0o755);
    }
    writeFileSync(versionFile, VERSION + "\n");
    writeFileSync(
      join(dest, "README.txt"),
      "ExifTool by Phil Harvey - https://exiftool.org\nFree software; you can redistribute it and/or modify it under the same terms as Perl itself (Artistic License or GPL).\n",
    );
    console.log(`ExifTool ${VERSION} installed in ${dest} (SHA-256 verified)`);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === normalize(process.argv[1])) {
  await main();
}
