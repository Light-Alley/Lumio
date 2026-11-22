#!/usr/bin/env node
// Prints the CHANGELOG.md section for one version (default: the version in
// package.json). The release workflow uses it as the release text, which Lumio shows
// in its update window. Usage: node scripts/release-notes.mjs [2.0.0]
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const version = process.argv[2] ?? JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;

/** The text under the "## [version]" (or "## version") heading, up to the next one. */
export function section(changelog, v) {
  const lines = changelog.split(/\r?\n/);
  const heading = (l) => /^##\s/.test(l);
  const escaped = v.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const mine = new RegExp(`^##\\s+\\[?v?${escaped}\\]?(\\s|\\(|$)`);
  const start = lines.findIndex((l) => mine.test(l));
  if (start < 0) return null;
  const rest = lines.slice(start + 1);
  const end = rest.findIndex(heading);
  return (end < 0 ? rest : rest.slice(0, end)).join("\n").trim();
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const text = section(readFileSync(join(root, "CHANGELOG.md"), "utf8"), version);
  if (!text) {
    console.error(`CHANGELOG.md has no section for ${version}.`);
    process.exit(1);
  }
  console.log(text);
}
