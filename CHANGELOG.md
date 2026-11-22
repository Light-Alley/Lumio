# Changelog

New versions are listed here automatically from the commit messages (see
[CONTRIBUTING.md](CONTRIBUTING.md)). Versions before 2.0.0 are described on the
[Releases page](https://github.com/Light-Alley/Lumio/releases).

## [2.0.0](https://github.com/Light-Alley/Lumio/compare/v1.0.1...v2.0.0) (2026-11-22)

A safety and reliability release, following a full review of Lumio's code.

### What's new

- **Safer with your files.** Lumio never follows links out of your library folders, never deletes the original when a folder move fails, and never replaces another file while renaming. Permanent deletion (on drives without a Trash) now picks photos strictly by the exact photo, never by name.
- **Your keys stay in the keychain.** API keys and the SmugMug sign-in are kept only in your system's keychain. Keys an older version saved in the catalogue are moved there and removed. If the keychain isn't available, Lumio says so instead of saving them elsewhere.
- **Location stays private when you ask.** Web exports and "smaller copies" for sharing now remove place names (city, region, country) as well as GPS coordinates, for videos too.
- **"Open with" is stricter.** Only apps your system lists for the file, or an app you pick yourself, can be used.
- **Fixes:** release links in the update window, watching libraries inside hidden folders, deleting folders with a dot in their name, sideways photos when copying to the clipboard, double-turned RAW previews, star ratings in the filmstrip, geotagging stacks from the map, and more.
- **Faster:** quicker scans of large and network folders, a much faster look-alike search, and smoother maps and selections in big libraries.

### Changed

- The AI assistant only connects to a custom server over https (or plain http on this computer).
- Installers are listed with checksums (SHA256SUMS.txt) and build provenance, and the README links straight to the latest download for each system.
