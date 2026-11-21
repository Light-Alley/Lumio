# Finding duplicates

**Find duplicates…** is in the sidebar under the library views; right-click a folder for
**Find duplicates here…**, which looks in that folder and its subfolders.

Lumio only *finds* duplicates. It never removes anything on its own:

- Each group shows every copy side by side, with its folder, size, pixel dimensions,
  date and rating, and one photo marked **Keep**: the largest, then the best rated and
  best described, then the one whose name doesn't look like a copy ("IMG_1 copy.jpg",
  "IMG_1 (2).jpg"), then the oldest file.
- You tick the photos you don't want. One photo in every group always stays: Lumio won't
  let you tick the last one.
- **Move to the Trash** asks once more, then sends the ticked photos (and their XMP
  sidecars) to the system Trash or Recycle Bin, where you can restore them. Drives without
  a Trash (some network shares) ask before deleting permanently.
- Photos on a drive that isn't connected are shown but can't be ticked.

## Exact copies

Files with exactly the same contents, whatever they are called or wherever they are.
For exact copies, every copy except the suggested one starts ticked, ready for you to
check.

To stay quick on network and cloud drives, Lumio only reads files that share their size
with another file. It first compares a fingerprint of each file's start and end, and
reads a file in full only when that matches too. The same file reached through two paths
(for example a link) is never counted as a copy of itself. Drives that aren't connected
are skipped.

## Look-alikes

The same picture saved more than once: resized, re-compressed, exported or lightly
edited. Choose how close they must be:

- **Same picture, resized or saved again** (the default)
- **Very similar**
- **Similar, such as burst shots**

Look-alikes start unticked, since they can be different shots you want to keep.

Lumio compares a 64-bit "difference hash" of each photo's thumbnail, which survives
resizing and re-compression, so this works with drives unplugged and doesn't read the
originals. Photos of a different shape (a crop, or portrait against landscape), RAW +
JPEG pairs of the same shot (same name, same folder) and nearly plain pictures (a black
frame, a clear sky) are left out. Each group is one photo and the photos close to it, so
a long run of slowly changing shots doesn't end up as one huge group.

Fingerprints are kept in the catalogue (`fingerprints` table) and reused until the file
changes, so a second search is much faster. The code is in `src-tauri/src/duplicates.rs`.
