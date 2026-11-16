// Downloads ExifTool (https://exiftool.org, by Phil Harvey) into src-tauri/resources/exiftool
// so it is bundled with the app. Run automatically before `tauri build`.
//
//   node scripts/fetch-exiftool.mjs            # for the current platform
//   node scripts/fetch-exiftool.mjs --windows  # force the Windows build
//
// macOS / Linux get the portable Perl version (Perl ships with both).
// Windows gets the stand-alone exiftool.exe build.
import { mkdirSync, existsSync, rmSync, writeFileSync, readdirSync, cpSync, chmodSync } from "node:fs";
import { join, dirname } from "node:path";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const dest = join(root, "src-tauri", "resources", "exiftool");
const windows = process.argv.includes("--windows") || process.platform === "win32";
const marker = join(dest, windows ? "exiftool.exe" : "exiftool");

if (existsSync(marker) && !process.argv.includes("--force")) {
  console.log(`ExifTool already present in ${dest}`);
  process.exit(0);
}

async function get(urls) {
  for (const u of urls) {
    try {
      const r = await fetch(u, { redirect: "follow", headers: { "User-Agent": "lumio-build" } });
      if (r.ok) return Buffer.from(await r.arrayBuffer());
      console.warn(`  ${u}: HTTP ${r.status}`);
    } catch (e) {
      console.warn(`  ${u}: ${e.message}`);
    }
  }
  throw new Error("Could not download ExifTool. Check your internet connection, or install it manually (see README).");
}

const version = (await get(["https://exiftool.org/ver.txt"])).toString().trim();
console.log(`Fetching ExifTool ${version} for ${windows ? "Windows" : "macOS/Linux"}…`);
const work = join(tmpdir(), `lumio-exiftool-${Date.now()}`);
mkdirSync(work, { recursive: true });

rmSync(dest, { recursive: true, force: true });
mkdirSync(dest, { recursive: true });

if (windows) {
  const name = `exiftool-${version}_64.zip`;
  const zip = await get([`https://exiftool.org/${name}`, `https://sourceforge.net/projects/exiftool/files/${name}/download`]);
  const zipPath = join(work, name);
  writeFileSync(zipPath, zip);
  execFileSync("tar", ["-xf", zipPath, "-C", work]); // bsdtar on Windows 10+ reads zip
  const dir = readdirSync(work).map((d) => join(work, d)).find((d) => existsSync(join(d, "exiftool_files")));
  if (!dir) throw new Error("Unexpected ExifTool zip layout");
  const exe = readdirSync(dir).find((f) => f.toLowerCase().endsWith(".exe"));
  cpSync(join(dir, "exiftool_files"), join(dest, "exiftool_files"), { recursive: true });
  cpSync(join(dir, exe), join(dest, "exiftool.exe"));
} else {
  const name = `Image-ExifTool-${version}.tar.gz`;
  const tgz = await get([`https://exiftool.org/${name}`, `https://sourceforge.net/projects/exiftool/files/${name}/download`]);
  const tgzPath = join(work, name);
  writeFileSync(tgzPath, tgz);
  execFileSync("tar", ["-xzf", tgzPath, "-C", work]);
  const dir = join(work, `Image-ExifTool-${version}`);
  cpSync(join(dir, "exiftool"), join(dest, "exiftool"));
  cpSync(join(dir, "lib"), join(dest, "lib"), { recursive: true });
  chmodSync(join(dest, "exiftool"), 0o755);
}
writeFileSync(join(dest, "VERSION"), version + "\n");
writeFileSync(
  join(dest, "README.txt"),
  "ExifTool by Phil Harvey - https://exiftool.org\nFree software; you can redistribute it and/or modify it under the same terms as Perl itself (Artistic License or GPL).\n",
);
rmSync(work, { recursive: true, force: true });
console.log(`ExifTool ${version} installed in ${dest}`);
