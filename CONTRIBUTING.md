# Contributing to Lumio

Thank you for wanting to help. Lumio is a small project looked after in spare time, so a little care up front saves everyone time.

## Ways to help

- **Report a bug** or **suggest an idea**: open an [issue](https://github.com/Light-Alley/Lumio/issues/new/choose). The templates ask for the things we always need (Lumio version, system, what drive the photos are on).
- **Test a release** on a system or camera we don't have, and say how it went.
- **Improve the docs**: the [user guide](docs/user-guide.md) and README are plain Markdown; small fixes can be made straight in GitHub.
- **Write code**: see below.

For anything bigger than a small fix, please open an issue first and say what you have in mind, so nobody spends a weekend on something that won't fit.

## What fits Lumio

Lumio aims to stay a **light, fast, safe photo browser**. Please keep these in mind:

- **Your photos come first.** Anything that touches files must be safe on external, network and cloud drives, which can disappear mid-operation. Never overwrite a file; copy and verify before removing an original; move to the Trash rather than deleting; keep sidecars with their photos; keep the catalogue in step with the disk. Changes to file operations need tests.
- **No pixel editing.** Lumio hands photos to the user's editor.
- **Small and dependency-light.** Use the system's own web view, image decoders and video support where possible. A new crate or npm package needs a good reason, a permissive licence (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0) and no big transitive tree.
- **Private by default.** No accounts, telemetry or analytics. Anything that talks to the internet must be something the user switched on, and the README must say so.
- **Standard metadata.** Ratings, labels, keywords and locations go into the files (or XMP sidecars) in the standard places so other apps can read them.
- **Plain language** in the interface and docs.

## Setting up

You need [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) 20 or later and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your system.

```bash
git clone https://github.com/Light-Alley/Lumio.git
cd Lumio
npm install
npm run fetch-exiftool   # downloads ExifTool into src-tauri/resources/exiftool
npm run app              # runs Lumio in development mode with live reload
```

Try your change with a **copy** of some photos, not your real library. Lumio's data folder (see the README) can be deleted at any time to start fresh.

## Before you open a pull request

```bash
npm run check                     # Svelte and TypeScript checks
npm run build                     # front end builds
cd src-tauri && cargo test        # Rust tests
```

The **Check** workflow runs the same on macOS, Windows and Linux for every pull request.

- Keep each pull request to one change, and describe what it does and how you tested it (which system, which kind of drive).
- Add or update tests for anything that moves, copies, renames, deletes or writes metadata.
- Update the README and [user guide](docs/user-guide.md) if users will notice the change.
- Don't bump the version number; that happens when a release is made.

## Where things are

The [README](README.md#how-its-built) lists the main Rust modules. In short: `src-tauri/src/` is the Rust back end (catalogue, indexing, thumbnails, file operations, metadata through ExifTool), and `src/` is the Svelte 5 + TypeScript front end, with state in `src/lib/store.svelte.ts`, actions and menus in `src/lib/actions.ts` and components in `src/lib/components/`.

## Licence

Lumio is MIT licensed. By sending a pull request you agree that your contribution is released under the same licence, and that you have the right to contribute it. Please don't copy code from projects with incompatible licences (for example GPL code from digiKam).

## Behaviour

Be kind and patient. See the [code of conduct](CODE_OF_CONDUCT.md).
