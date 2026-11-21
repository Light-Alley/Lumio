# Lumio user guide

Lumio is a photo browser. It shows the photos that are already in your folders, on this computer, an external drive or a network drive, and helps you sort, rate, describe and move them. It never imports or copies your library, and it never edits pixels.

This guide takes about ten minutes to read. Shortcuts are written for a Mac (`⌘`); on Windows and Linux use `Ctrl` instead.

## 1. Before you start: keep a backup

Lumio moves, renames and deletes real files when you ask it to, and writes ratings and keywords into them. It is careful (moves between drives are copied and checked before the original is removed, nothing is ever overwritten, and deleted photos go to the Trash or Recycle Bin), but no software is perfect. Keep a backup of your photos, as you would with any program that manages them.

## 2. Install

Download the installer for your computer from the [Releases page](https://github.com/LIght-Alley/Lumio/releases/latest):

| System | File |
|---|---|
| Mac with Apple silicon (M1 and later) | `Lumio_x.y.z_aarch64.dmg` |
| Mac with an Intel processor | `Lumio_x.y.z_x64.dmg` |
| Windows 10 or 11 | `Lumio_x.y.z_x64-setup.exe` (or the `.msi`) |
| Linux | `.AppImage` (any distribution), `.deb` (Ubuntu, Debian, Mint) or `.rpm` (Fedora, openSUSE) |

On a Mac, open the `.dmg` and drag Lumio into Applications. If you're not sure which Mac you have, choose  → About This Mac and look at "Chip" or "Processor".

If your system warns about an unknown developer, see [Installing](../README.md#installing) in the README.

## 3. Add your photos

1. Click **Add a folder** (or the **+** next to *Library* in the sidebar).
2. Choose a folder of photos. Lumio includes all the folders inside it.
3. Lumio starts cataloguing straight away. The status bar at the bottom shows its progress; you can keep working while it runs.

A big library on a network or cloud drive takes a while the first time (Lumio reads each photo once to make its thumbnail). After that, opening a folder is instant.

You can add as many folders as you like, from different drives. To stop Lumio showing a folder, right-click it and choose **Remove from library…**. That only removes it from Lumio's catalogue; the photos stay where they are.

### Drives that come and go

Lumio remembers external, network and cloud drives even when they're unplugged. Their folders stay in the sidebar, greyed out, and you can still browse, search, rate and view the photos, because Lumio keeps thumbnails and (for these drives) a sharp preview of every photo on your computer. Changes you make are written to the files when the drive comes back.

Choose how sharp those previews are in **Settings → General → Previews for offline viewing**: Normal (1600 px, about 0.3 MB a photo), High (2560 px, sharp full screen, about three times the space), or Off.

## 4. Find your way around

- **Sidebar (left):** smart views (*All photos*, *Recently added*, *Rated*, *On the map*, *Without location*) and your folders. Drag its edge to make it wider; double-click the edge to reset it.
- **Toolbar (top):** search, sort, thumbnail size, view switch, the info panel, *Ask AI*, slideshow, share and settings.
- **Info panel (right):** the selected photo or its histogram, rating, label, and metadata you can edit. Hide it with `I` or the tab on its edge.
- **Menu bar:** every command is in the menus (File, Edit, View, Photo, Window, Help), with its shortcut beside it. Commands in the Photo menu act on the selected photos, or on the photo open in the viewer. **Help → Keyboard Shortcuts** lists all the keys.
- **Tabs:** `⌘T` opens a new tab, `⌘W` closes one, `⌘1`…`⌘9` switch. Drag photos onto another tab to move them into that tab's folder.

### Views

| Key | View |
|---|---|
| `G` | Grid of thumbnails |
| `L` | List with sortable columns |
| `F` | Filmstrip: one large photo above a strip of thumbnails |
| `M` | Map of photos that have a location |

Press `Space` or `Enter` to open the focused photo in the viewer. Use the arrow keys to go through photos, `Z` to zoom, `B` to change the background (black, grey, light grey, white), `F` for full screen and `Esc` to go back.

### Search and filter

Type in the search box to find photos by file name, keyword, caption, place, camera or lens. Use the rating and colour label filters next to it to narrow things down. Tick **Include photos in subfolders** to see everything below the selected folder at once.

### Ask AI

Click **Ask AI** and describe what you're looking for, such as "sunsets in Scotland from 2023 with four stars or more". The assistant searches the catalogue for you. It needs either a Claude API key (paid by use, a fraction of a cent per search) or a free model running on your own computer with Ollama or LM Studio. Set it up in **Settings → AI assistant**; the [README](../README.md#using-the-ai-finder) has the details. Nothing is sent anywhere until you set it up.

## 5. Sort and describe

Select one photo, or several with `⌘`-click or `Shift`-click (`⌘A` selects all), then:

| Key | Does |
|---|---|
| `1`–`5` | Star rating (`0` removes it) |
| `6`, `7`, `8`, `9` | Red, yellow, green, blue colour label (press again to remove) |
| `⌘G` | Group shots of the same subject (`⇧⌘G` ungroups); `S` opens or closes a group |
| `⌘[` / `⌘]` | Rotate left or right |
| `F2` | Rename (several photos: pattern or find-and-replace) |

Ratings, labels, titles, captions, keywords, creator and copyright are written into the photo as standard metadata, so Lightroom, Bridge, Capture One, darktable and digiKam see them too. For RAW files they go into a small `.xmp` "sidecar" file next to the photo by default, so the RAW file itself is never changed (you can change this in **Settings → General → RAW files**).

Groups are kept only in Lumio's catalogue.

## 6. Organise files

- **Move:** drag photos onto a folder in the sidebar or onto another tab. Hold `⌥` (`Alt`) while dropping to copy instead. Or right-click and use **Move to…** / **Copy to…**.
- **New folder, rename folder, move folder to Trash:** right-click a folder.
- **Delete:** `⌘Backspace` or `Delete` moves photos to the Trash (Recycle Bin on Windows). Sidecar files go with them.
- **Rename many photos:** select them and press `F2`. Build a name from parts like `{date} Iceland {n}`, see a preview of every new name, then click Rename. **Undo** in the message that appears puts the old names back.

Moving to or from a drive that isn't connected is allowed: the photos show in their new folder at once and the files follow when the drive is back. The status bar shows how many moves are waiting; click it to see or cancel them.

## 7. Location and the map

Press `M` for the map. Drag photos from the filmstrip onto the map to give them a location, or select photos and click a spot. You can search for a place by name. Locations are written into the files. The map loads its background from OpenStreetMap; that's the only thing Lumio fetches from the internet unless you use the AI finder, sharing or SmugMug.

## 8. Show and share

- **Slideshow:** press `P` (or the slideshow button). It plays the selected photos, or the whole view if nothing is selected. `Space` pauses, arrow keys go back and forward, `+`/`-` change the speed, `S` shuffles, `F` toggles full screen and `Esc` ends it. Set the default timing in the slideshow controls.
- **Share:** right-click → **Share** to use the system share menu (AirDrop, Messages, Mail), email, WhatsApp, or a website. RAW and HEIC photos are sent as JPEG copies. **Settings → General → Sharing** can make smaller copies that leave out the location.
- **SmugMug:** right-click → **Share → Upload to SmugMug…**. You need a free SmugMug API key; the window walks you through it ([details](smugmug.md)).
- **Web-ready copies:** right-click → **Export web-ready copies…** makes resized JPEGs in a folder you choose, optionally without the GPS location.

## 9. Videos

Videos appear next to your photos, with their length on the thumbnail. Open one to play it: `Space` or `K` play and pause, `J` and `L` jump 10 seconds, `M` mutes. Which formats play depends on your system ([details](video.md)).

## 10. Editing in another app

Lumio doesn't edit pixels. Right-click a photo → **Open with** to open it in Photoshop, Affinity Photo, GIMP, Preview or any other app. When that app saves or exports into one of your library folders, Lumio notices and updates the thumbnail, or shows the new file.

## 11. Where your data lives

Lumio keeps a catalogue (`catalogue.sqlite`), thumbnails, previews and a log (`lumio.log`) in:

- macOS: `~/Library/Application Support/app.lumio.photos/`
- Windows: `%APPDATA%\app.lumio.photos\`
- Linux: `~/.local/share/app.lumio.photos/`

Deleting that folder resets Lumio. Your photos, and everything written into them, stay. API keys and the SmugMug sign-in are kept in your system keychain, not in that folder.

## 12. Updates

Lumio tells you when a new version is out: a notice appears in the corner a few seconds after it opens. Click **See what's new** to read the release notes, then **Install and Restart** (or **Download**, which opens the release page so you can install the new version over the old one). Your library, settings and photos are kept. **Lumio → Check for Updates…** (on Windows and Linux, **Help → Check for Updates…**) checks straight away. To stop the automatic check, untick it in **Settings → General → Updates**.

## 13. Troubleshooting

| Problem | Try this |
|---|---|
| A folder is greyed out | The drive isn't connected or isn't answering. Connect it, or wait: Lumio checks again every minute and when the window comes to the front. |
| Indexing stopped part-way | Usually the computer slept or the network drive dropped out. Lumio pauses and carries on when the drive answers again. If it seems stuck, quit and reopen Lumio. |
| A thumbnail looks wrong or old | Right-click → **Re-read metadata & thumbnail**. For a whole folder, right-click it → **Rescan folder**. |
| A RAW file shows "file not readable" | Some older or unusual RAW files have no preview Lumio can read. Open it in your editor, or [open an issue](https://github.com/LIght-Alley/Lumio/issues) with the camera model. |
| Something else | Open an [issue](https://github.com/LIght-Alley/Lumio/issues) and attach `lumio.log` from the folder above. Remove anything in it you'd rather not share. |

## Keyboard shortcuts

| Where | Key | Does |
|---|---|---|
| Anywhere | `G` `L` `F` `M` | Grid, list, filmstrip, map |
| | `⌘F`, `⌘O`, `⇧⌘N` | Search, add a folder, new folder |
| | `⌘=` / `⌘-` | Larger or smaller thumbnails |
| | `⌘,`, `⌘/` | Settings, list of shortcuts |
| | `⌃⌘F` (Windows, Linux: `F11`) | Full screen |
| | `I` | Show or hide the info panel |
| | `P` | Slideshow |
| | `⌘T` / `⌘W` / `⌘1`–`⌘9` | New tab, close tab, switch tab |
| Browsing | arrows, `Home`, `End` | Move through photos (`Shift` extends the selection) |
| | `⌘A`, `Esc` | Select all, clear selection |
| | `Space` / `Enter` | Open in the viewer |
| | `0`–`5` | Rating |
| | `6`–`9` | Colour label |
| | `⌘G`, `⇧⌘G`, `S` | Group, ungroup, open or close group |
| | `⌘[`, `⌘]` | Rotate |
| | `F2` | Rename |
| | `⌘C` | Copy photos to the clipboard |
| | `Delete` / `⌘Backspace` | Move to Trash |
| Viewer | arrows | Previous / next |
| | `Z`, `B`, `F`, `I` | Zoom, background, full screen, info |
| | `Esc` | Leave full screen, then close |
| Video | `Space`/`K`, `J`, `L`, `M` | Play/pause, back 10 s, forward 10 s, mute |
| Slideshow | `Space`, arrows, `+`/`-`, `S`, `F`, `I`, `Esc` | Pause, previous/next, speed, shuffle, full screen, caption, end |
