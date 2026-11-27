# Frameloft 1.0 plan

Frameloft grew quickly through the 0.3.x betas. Version 1.0 is the point where it's safe to trust with a whole library and ready for strangers to install. After 1.0, new features go into 1.x releases (1.1, 1.2…) and fixes into patch releases (1.0.1…), and anything that could break someone's catalogue or files waits for a 2.0.

## What 1.0 means

1. **Safe for originals.** File operations (move, copy, rename, trash, metadata writes) have automated tests, including drives disappearing mid-operation, and there's a clear way to undo recent changes.
2. **Edits travel with the files.** Ratings, colour labels and keywords are written to XMP so Lightroom, Capture One and others read them, and Frameloft reads back what those apps wrote.
3. **Manageable at 50,000+ photos.** Duplicate finding and smart collections (for example "5 stars from 2024").
4. **Installs cleanly.** macOS builds are signed and notarised; Windows builds are signed if possible.
5. **Documented and open.** User guide, contributing guide, code of conduct, security policy, issue templates, public repository.

## Checklist

### Features and safety (work in progress in separate pull requests)

- [x] File-safety tests and undo history merged
- [ ] XMP ratings, labels and keywords merged (done), round-trip checked with at least one of Lightroom, Bridge, Capture One or darktable
- [x] Duplicate finding and smart collections merged
- [ ] No open issues labelled `bug` that can lose or damage photos

### Signing (needs Liju, see [code-signing.md](code-signing.md))

- [ ] Apple Developer Program membership active
- [ ] `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` secrets added (and `APPLE_SIGNING_IDENTITY` if wanted)
- [ ] A test run of **Build installers** shows "signing" and "notarising" in both macOS jobs, and `spctl -a -vvv -t install` says "Notarized Developer ID" on a downloaded build
- [x] Decision (2026-11-21): ship 1.0 unsigned on macOS and Windows; revisit the Apple Developer Program and SignPath Foundation after launch
- [ ] README "Installing" section updated to match (drop the macOS `xattr` workaround once notarised)

### Release candidate testing

Build `1.0.0-rc.1` with the workflow and test on real machines, with a **copy** of part of a library first, then the full library:

- [ ] macOS Apple silicon: install over 0.3.x, catalogue and settings kept
- [ ] macOS Intel (or Rosetta at minimum)
- [ ] Windows 11: install, add a folder on a USB drive and a network share
- [ ] Linux: AppImage and `.deb` on Ubuntu 24.04
- [ ] Full 55,000-photo pCloud library: first index from scratch, sleep/wake during indexing, unplug/replug
- [ ] Move, copy, rename (single and batch), trash, undo, on local, external and network drives, including with the drive unplugged (queued moves)
- [ ] Ratings/labels/keywords written and read back in another app, for JPEG, HEIC and at least two RAW formats (with sidecars)
- [ ] Viewer, filmstrip, full screen, slideshow, video playback, histogram, map and geotagging
- [ ] Share menu, email, SmugMug upload (live account), web-ready export
- [ ] AI finder with Claude and with Ollama
- [ ] Upgrade path: open a 0.3.8 catalogue with 1.0 and check nothing is lost
- [x] Run `cargo audit` and `npm audit --omit=dev` and fix anything serious

### Open-source readiness

- [x] MIT `LICENSE` with copyright "Light and Alley and Frameloft contributors"; same wording in the app bundle and `Cargo.toml`
- [x] Dependency licences checked: all 662 Rust crates and the npm runtime packages are MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, CC0, CDLA-Permissive or MPL-2.0 (unmodified), all compatible with MIT. ExifTool is bundled unmodified under its own Perl licence, with its notice in `resources/exiftool/README.txt`
- [x] `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, issue and pull request templates, user guide
- [ ] Decide whether the "Light and Alley" name and logo are licensed with the code or reserved (common choice: code is MIT, name and logo are not, so forks must rename). If reserved, add a line to the README
- [ ] Check the git history has no secrets or private photos before making the repository public (GitHub → Settings → Code security → secret scanning also helps)
- [ ] Repository settings: make public; turn on private vulnerability reporting, Discussions (optional), and branch protection for `main` requiring the **Check** workflow
- [ ] Add labels `bug`, `idea`, `good first issue`, `help wanted`
- [ ] Optional: rename the organisation from "Light-Alley" to "Light-Alley" **before** going public, so the address that gets shared is the final one. GitHub redirects the old address, but links in the README and guide should be updated

### Release

- [ ] Bump the version to `1.0.0` in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` (and `package-lock.json`, which has been stuck at 0.1.0)
- [ ] Write release notes: what Frameloft is, the headline features, what changed since 0.3.8, known limitations, and thanks
- [ ] Run **Build installers**, check every asset is in the draft release (two macOS `.dmg`, Windows `.msi` and `-setup.exe`, Linux `.AppImage`, `.deb`, `.rpm`)
- [ ] Download each installer once and open it
- [ ] Publish the release (only Liju presses Publish), named "Frameloft 1.0"
- [ ] Update the README screenshot and the promo video links; start promoting

## After 1.0

Ideas to consider, in no particular order: resizable right panel, face or object search with a local model, Flatpak or Homebrew cask, translations, and published docs on GitHub Pages.

## Making a release (any version)

1. Merge the pull requests for the release into `main`.
2. Bump the version in the three files above, in one commit.
3. Actions → **Build installers** → **Run workflow** on `main`. All four jobs (two macOS, Windows, Linux) should go green. If a macOS job sits in the queue for a long time, cancel and re-run it; if the Linux job fails downloading ExifTool, re-run it.
4. Open the draft release, edit the notes, and publish. Publishing creates the tag, which runs the workflow once more and re-uploads the same files; that's harmless.
