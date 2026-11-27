# Update notices and in-app updates

Frameloft tells people when a newer version is out, and can install it for them.

## What people see

- About eight seconds after Frameloft opens (and at most twice a day after that), Frameloft asks GitHub for the latest **published** release. Drafts and pre-releases are ignored. If it's newer than the running copy, a notice appears in the corner: *"Frameloft 1.1.0 is available"*, with **See what's new**.
- **Frameloft → Check for Updates…** (on Windows and Linux: **Help → Check for Updates…**, also in **Settings → General → Updates** and **Settings → About**) checks straight away and says "You have the latest version" when there's nothing new.
- The update window shows the release notes and offers:
  - **Install and Restart**, when the release was signed with Frameloft's updater key (below). Frameloft downloads the update, checks its signature, installs it and restarts.
  - **Download** otherwise, which opens the release page so the installer can be downloaded by hand.
  - **Skip This Version** (no more automatic notices for that version) and **Later**.
- Automatic checks can be switched off in **Settings → General → Updates**.

The check is a plain HTTPS request to `api.github.com`. It sends nothing about the person or their photos. GitHub sees the computer's IP address, as with any web page. The repository has to be **public** for the check to work, because GitHub answers "not found" for private repositories.

Linux `.deb` and `.rpm` installs always get **Download**, since the package manager owns those files. The AppImage can update itself.

## The updater key (for Install and Restart)

Tauri's updater only installs a download that is signed with a private key you hold, and checks it against the public key built into Frameloft. This stops anyone who can tamper with a download (or with the GitHub release) from pushing a different app to people.

This key is **not** an Apple Developer ID or a Windows code-signing certificate. It is free, you make it yourself in a minute, and it works whether or not the installers are code-signed. Code signing ([code-signing.md](code-signing.md)) is about the warning macOS and Windows show on first launch; the updater key is about Frameloft trusting its own updates.

### Step 1: make the key (once, on your Mac)

In Terminal, inside the Frameloft source folder:

```bash
npx tauri signer generate -w ~/.tauri/lumio-updater.key
```

It asks for a password (choose a strong one and keep it in your password manager). It writes two files:

- `~/.tauri/lumio-updater.key`: the **private** key. Never commit it or share it.
- `~/.tauri/lumio-updater.key.pub`: the **public** key. Safe to share; it goes into every build.

**Back up both files and the password.** If the private key is lost, copies of Frameloft that are already installed can't install updates signed with a new key; people would have to download the next version by hand once.

### Step 2: add them to GitHub

In **GitHub → frameloft/Frameloft → Settings → Secrets and variables → Actions**:

| Where | Name | Value |
|---|---|---|
| Secrets tab, **New repository secret** | `TAURI_SIGNING_PRIVATE_KEY` | the whole contents of `lumio-updater.key` (`cat ~/.tauri/lumio-updater.key \| pbcopy`) |
| Secrets tab, **New repository secret** | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password from step 1 |
| **Variables** tab, **New repository variable** | `LUMIO_UPDATER_PUBKEY` | the whole contents of `lumio-updater.key.pub` (`cat ~/.tauri/lumio-updater.key.pub \| pbcopy`) |

The public key is a *variable*, not a secret, because it isn't secret and the build needs to read it.

### Step 3: release as usual

The next run of **Build installers** sees the key and:

- builds each installer with an update package next to it (`.app.tar.gz` for macOS, the setup `.exe`/`.msi` for Windows, `.AppImage` for Linux) plus a `.sig` signature file,
- writes `latest.json`, which lists the new version and where each package is, and attaches it to the draft release,
- builds the public key and the address of `latest.json` (`https://github.com/frameloft/Frameloft/releases/latest/download/latest.json`) into the app.

When you publish the draft, copies of Frameloft **built with the key** offer **Install and Restart**. Copies with the update check but built before the key existed still get the notice, and their **Download** button opens the release page; once people have installed one keyed version, later updates are one click. (Lumio 1.0.0 as first built has no update check at all, so people on it only find out from the website.)

The run log says "In-app updates: on" or shows a warning when the key isn't set. Nothing breaks without it.

## Updating from Lumio

Frameloft was called Lumio until version 2.0.0, and its GitHub address was Light-Alley/Lumio. Copies of Lumio already installed keep working and keep getting updates:

- Their update check and updater still ask the old address. GitHub forwards both the release API and `releases/latest/download/latest.json` to frameloft/Frameloft, so they find Frameloft's releases. **Never create a new repository at Light-Alley/Lumio**, or that forwarding stops.
- The bundle identifier is still `app.lumio.photos` (and the keychain entry is still "Lumio Photo Manager"), so the catalogue, thumbnails, settings and saved keys are the same ones after the update. Don't change either.
- The updater key and the `LUMIO_UPDATER_PUBKEY` variable are unchanged, so updates signed for Frameloft install in Lumio.

What changes for people on each system:

- **macOS**: Install and Restart replaces the app where it is, so it may still be called *Lumio.app* in Applications while showing Frameloft inside. Installing from the new `.dmg` adds *Frameloft.app*; the old *Lumio.app* can then be moved to the Bin.
- **Windows**: the installer puts Frameloft in its own folder, so *Lumio* stays in **Settings → Apps** until it's removed. When uninstalling Lumio, leave **Delete the application data** unticked: Frameloft uses the same data folder.
- **Linux**: the `frameloft` `.deb` and `.rpm` replace the old `lumio` package. An AppImage is replaced in place by Install and Restart.
- **MSI**: the upgrade code is fixed in `tauri.conf.json` to Lumio's, so the Frameloft `.msi` upgrades an installed Lumio `.msi`.

## Where the code is

- `src-tauri/src/updates.rs`: the GitHub check, version comparison, and the install command.
- `src/lib/updates.svelte.ts`: when to check, the notice, skip and install.
- `src/lib/components/UpdateDialog.svelte`: the update window.
- `.github/workflows/release.yml`, "Code signing setup" step: turns the updater on when the key is set.
