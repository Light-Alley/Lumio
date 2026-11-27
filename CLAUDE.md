# Working on Frameloft

Frameloft is a Tauri 2 desktop app: a Rust back end in `src-tauri/` and a Svelte 5 + TypeScript front end in `src/`. Start with [README.md](README.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## Before you finish

- Run `npm run verify` (repository rules, svelte-check, Vitest, build, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`). Fetch ExifTool first (`npm run fetch-exiftool`) so its tests run; CI sets `FRAMELOFT_REQUIRE_EXIFTOOL=1` and fails if they would skip.
- After a failure, re-run only what failed while fixing it.

## Rules

- Commit messages use Conventional Commits (`fix:`, `feat:`, `docs:`, `chore(deps):`; `!` for breaking changes). Releases and CHANGELOG.md are made from them by release-please; don't edit versions by hand except through its release pull request, and keep the five version files in step (`package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`).
- Never switch a check off (`#[allow]`, `#[expect]`, `#[ignore]`, `@ts-ignore`, `@ts-expect-error`, `svelte-ignore`, `.skip`/`.only`, `continue-on-error`, `|| true`). Fix the cause. `scripts/verify-repo-rules.mjs` enforces this.
- Keep every source file under 1,000 lines; split by topic when one grows.
- Prefer type guards over `as T` casts for data from outside (JSON, IPC, the network).
- Files come first: never overwrite a file, copy and verify before removing an original, keep sidecars with their photos, and add tests for anything that moves, copies, renames, deletes or writes metadata.
- Paths from the window are untrusted: check them against the library folders (`commands::in_library`) before touching the disk. Secrets live only in the OS keychain (`keys.rs`).
- ExifTool arguments go through `exiftool::path_arg` / `fmt_path_arg` / `encode_arg`, never raw strings.
- GitHub Actions are pinned to full commit SHAs, with `permissions:` kept minimal and `persist-credentials: false`. Release builds use no caches.
- The project address is `https://github.com/frameloft/Frameloft`. The copyright line is "Light and Alley and Frameloft contributors".
