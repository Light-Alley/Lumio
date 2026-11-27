#!/usr/bin/env node
// Repository rules that no other tool checks. Run with `npm run verify:rules` (part of
// `npm run verify` and CI). Prints every broken rule and exits non-zero if there is one.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, normalize, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const files = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" })
  .split("\0")
  .filter((f) => f && existsSync(join(root, f)));
const read = (f) => readFileSync(join(root, f), "utf8");
const problems = [];
const report = (file, line, message) => problems.push(`${file}${line ? `:${line}` : ""}: ${message}`);

const SOURCE = /\.(rs|ts|js|mjs|svelte)$/;
const sources = files.filter((f) => SOURCE.test(f) && !f.startsWith("build/") && f !== "package-lock.json");
const self = relative(root, fileURLToPath(import.meta.url));

// 1. No switched-off checks. (Spelled in pieces so this file doesn't match itself.)
const SWITCH_OFFS = [
  "#[" + "allow(", "#![" + "allow(", "#[" + "expect(", "#[" + "ignore",
  "@ts-" + "expect-error", "@ts-" + "ignore", "svelte-" + "ignore", "biome-" + "ignore", "eslint-" + "disable",
  "it." + "skip(", "it." + "only(", "describe." + "only(", "test." + "only(",
  "continue-on-error: " + "true", "|| " + "true", "nosem" + "grep", "gitleaks" + ":allow",
];
const CHECKED = files.filter((f) => (SOURCE.test(f) || /\.(ya?ml|toml|json)$/.test(f) || f.startsWith(".githooks/")) && f !== self && f !== "package-lock.json" && !f.startsWith("build/"));
for (const f of CHECKED) {
  read(f).split("\n").forEach((line, i) => {
    for (const s of SWITCH_OFFS) if (line.includes(s)) report(f, i + 1, `"${s}" switches a check off; fix the cause instead`);
  });
}

// 2. Source files stay under 1,000 lines, so each can be read as a whole.
for (const f of sources) {
  const n = read(f).split("\n").length;
  if (n > 1000) report(f, 0, `${n} lines; split it (limit 1,000)`);
}

// 3. No reminder comments left behind: open an issue instead.
const REMINDER = new RegExp("\\b(" + ["TO" + "DO", "FIX" + "ME", "HA" + "CK", "X" + "XX"].join("|") + ")\\b");
for (const f of sources.filter((x) => x !== self)) {
  read(f).split("\n").forEach((line, i) => {
    if (REMINDER.test(line)) report(f, i + 1, "reminder comment; open an issue instead");
  });
}

// 4. Markdown: internal links point at files that exist, and every doc is linked from somewhere.
const md = files.filter((f) => f.endsWith(".md"));
const linked = new Set();
const LINK = /\]\(([^)\s]+)\)/g;
for (const f of md) {
  const text = read(f);
  for (const m of text.matchAll(LINK)) {
    const target = m[1].split("#")[0];
    if (!target || /^[a-z]+:/i.test(target)) continue;
    const resolved = normalize(join(dirname(f), decodeURIComponent(target)));
    if (!existsSync(join(root, resolved))) report(f, 0, `broken link to ${m[1]}`);
    else linked.add(resolved);
  }
}
for (const f of md.filter((x) => x.startsWith("docs/"))) {
  if (!linked.has(normalize(f))) report(f, 0, "not linked from README.md or another document");
}

// 5. Links use the project's current address (CHANGELOG.md keeps its history as written).
const OLD_ADDRESSES = ["LI" + "ght-Alley", "Light-Alley" + "/Lumio", "alahmedmadina" + "/Lumio"];
for (const f of files.filter((x) => !x.startsWith("build/") && x !== self && x !== "CHANGELOG.md" && !/\.(png|jpe?g|icns|ico|gif|webp)$/i.test(x))) {
  const text = read(f).toLowerCase();
  for (const old of OLD_ADDRESSES) {
    if (text.includes(`github.com/${old.toLowerCase()}`)) report(f, 0, `link to the old address github.com/${old}; use https://github.com/frameloft/Frameloft`);
  }
}

// 6. One version number everywhere.
const versions = {
  "package.json": JSON.parse(read("package.json")).version,
  "package-lock.json": JSON.parse(read("package-lock.json")).version,
  "src-tauri/tauri.conf.json": JSON.parse(read("src-tauri/tauri.conf.json")).version,
  "src-tauri/Cargo.toml": /^version\s*=\s*"([^"]+)"/m.exec(read("src-tauri/Cargo.toml"))?.[1],
};
if (new Set(Object.values(versions)).size !== 1) {
  report("", 0, `versions differ: ${Object.entries(versions).map(([k, v]) => `${k} ${v}`).join(", ")}`);
}

if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} rule${problems.length === 1 ? "" : "s"} broken.`);
  process.exit(1);
}
console.log(`Repository rules: OK (${files.length} files).`);
