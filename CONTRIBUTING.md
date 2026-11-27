# Contributing to Frameloft

Thank you for wanting to help. Frameloft is a small project looked after in spare time, so a little care up front saves everyone time.

## Ways to help

- **Report a bug** or **suggest an idea**: open an [issue](https://github.com/frameloft/Frameloft/issues/new/choose). The templates ask for the things we always need (Frameloft version, system, what drive the photos are on).
- **Test a release** on a system or camera we don't have, and say how it went.
- **Improve the docs**: the [user guide](docs/user-guide.md) and README are plain Markdown; small fixes can be made straight in GitHub.
- **Write code**: see below.

For anything bigger than a small fix, please open an issue first and say what you have in mind, so nobody spends a weekend on something that won't fit.

## What fits Frameloft

Frameloft aims to stay a **light, fast, safe photo browser**. Please keep these in mind:

- **Your photos come first.** Anything that touches files must be safe on external, network and cloud drives, which can disappear mid-operation. Never overwrite a file; copy and verify before removing an original; move to the Trash rather than deleting; keep sidecars with their photos; keep the catalogue in step with the disk. Changes to file operations need tests.
- **No pixel editing.** Frameloft hands photos to the user's editor.
- **Small and dependency-light.** Use the system's own web view, image decoders and video support where possible. A new crate or npm package needs a good reason, a permissive licence (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0) and no big transitive tree.
- **Private by default.** No accounts, telemetry or analytics. Anything that talks to the internet must be something the user switched on, and the README must say so.
- **Standard metadata.** Ratings, labels, keywords and locations go into the files (or XMP sidecars) in the standard places so other apps can read them.
- **Plain language** in the interface and docs.

## Setting up

You need [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) 20 or later and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your system.

```bash
git clone https://github.com/frameloft/Frameloft.git
cd Frameloft
npm install
npm run fetch-exiftool   # downloads ExifTool into src-tauri/resources/exiftool
npm run app              # runs Frameloft in development mode with live reload
```

Try your change with a **copy** of some photos, not your real library. Frameloft's data folder (see the README) can be deleted at any time to start fresh.

## Before you open a pull request

```bash
npm run verify    # everything below, in one go
```

`npm run verify` runs the repository rules (`scripts/verify-repo-rules.mjs`), the Svelte and TypeScript checks, the front-end tests (Vitest), the front-end build, `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test`. To run it automatically before every push, turn on the hook once:

```bash
git config core.hooksPath .githooks
```

The **Check** workflow runs the same on macOS, Windows and Linux for every pull request, plus dependency advisories (npm audit, cargo audit, OSV-Scanner), a secret scan of the whole history (gitleaks), CodeQL, and lint for the workflows (actionlint, zizmor).

The repository rules keep a few habits: no switched-off checks (`#[allow]`, `@ts-ignore`, `svelte-ignore` and the like; fix the cause instead), no source file over 1,000 lines, no reminder comments (open an issue), every document linked from somewhere and every internal link working, and one version number in all manifests.

### Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org): `fix: keep sidecars when renaming`, `feat: slideshow shuffle`, `docs: …`, `chore(deps): …`. A `!` after the type (`feat!:`) or a `BREAKING CHANGE:` line marks a change people must know about. Releases are made from these messages (see below), so they are what the changelog says.

- Keep each pull request to one change, and describe what it does and how you tested it (which system, which kind of drive).
- Add or update tests for anything that moves, copies, renames, deletes or writes metadata.
- Update the README and [user guide](docs/user-guide.md) if users will notice the change.
- Don't bump the version number; that happens when a release is made.

## How releases are made

1. Every push to `main` updates a pull request called **chore(main): release X.Y.Z** (the Release workflow, using release-please). It sets the new version in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src-tauri/tauri.conf.json`, and adds the changes to [CHANGELOG.md](CHANGELOG.md). Edit the CHANGELOG text in that pull request if it needs plainer words: the new version's section is what Frameloft shows in its update window.
2. Merging it creates a **draft** release and builds the installers for every system (`.github/workflows/release.yml`), with `SHA256SUMS.txt`, build attestations, and copies under fixed names for the README download links.
3. Check the draft, then press **Publish**. Installed copies of Frameloft see the new version from then on.

The repository setting **Settings → Actions → General → Allow GitHub Actions to create and approve pull requests** must be on for step 1.

## Where things are

The [README](README.md#how-its-built) lists the main Rust modules. In short: `src-tauri/src/` is the Rust back end (catalogue, indexing, thumbnails, file operations, metadata through ExifTool), and `src/` is the Svelte 5 + TypeScript front end, with state in `src/lib/store.svelte.ts`, actions and menus in `src/lib/actions.ts` and components in `src/lib/components/`.

## Licence

Frameloft is MIT licensed. By sending a pull request you agree that your contribution is released under the same licence, and that you have the right to contribute it. Please don't copy code from projects with incompatible licences (for example GPL code from digiKam).

## Behaviour

Be kind and patient. See the [code of conduct](CODE_OF_CONDUCT.md).
