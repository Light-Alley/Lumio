# Ratings, colour labels and keywords

Frameloft stores ratings, colour labels and keywords as standard XMP metadata, the same
tags Adobe Lightroom Classic, Bridge, Capture One, digiKam and darktable use. Nothing
lives only in Frameloft's catalogue: if you delete the catalogue, or open the photos in
another app, the stars, labels and keywords are still there.

## Shortcuts

Select one or more photos in the grid, list or filmstrip (or open one in the viewer):

| Key | Does |
| --- | --- |
| `1`–`5` | Rate 1 to 5 stars |
| `0` | Remove the rating |
| `6` `7` `8` `9` | Red, yellow, green, blue label (press again to remove it) |

Purple is in the right-click menu and the info panel, as in Lightroom, where it has no key.

## Keywords

Add keywords in the info panel (`I`). Separate several with commas. As you type, Frameloft
suggests keywords you already use, most used first.

For a keyword inside others, type the path with `>` (or `|`):
`Places > France > Paris`. Frameloft saves it the way Lightroom does:

- `dc:subject` (and IPTC Keywords inside JPEG and TIFF files) gets `Places`, `France` and `Paris`;
- `lr:hierarchicalSubject` gets `Places|France|Paris`.

Keyword paths made in Lightroom or Capture One are kept when you edit a photo in Frameloft.
When you remove a keyword, Frameloft also removes the paths that contain it, and their parent
keywords if nothing else needs them; otherwise Lightroom would bring the keyword back the
next time it reads the file.

## Where Frameloft writes

| Photo | Default | Setting |
| --- | --- | --- |
| RAW (CR2, CR3, NEF, ARW, DNG, RAF, ORF…) | XMP sidecar next to the RAW file. The RAW file is never changed. | Settings › General › RAW files |
| JPEG, HEIC, TIFF, PNG, WebP | Inside the file. ExifTool rewrites only the metadata, not the picture. | Settings › General › JPEG, HEIC, TIFF and other photos |
| MP4, MOV | Inside the file (XMP) | same |
| MKV, AVI and other videos | XMP sidecar | — |

Choose **XMP sidecar file** for JPEG and other photos if you want Frameloft never to change any
original. Capture One and other apps that read sidecars pick these up, but Lightroom
and Bridge ignore sidecars for JPEG files.

Sidecars are named the way Lightroom names them: `IMG_0001.xmp` next to `IMG_0001.CR2`.
Frameloft also reads darktable-style `IMG_0001.CR2.xmp`. When a JPEG has a RAW twin with the
same name, `IMG_0001.xmp` belongs to the RAW file, so the JPEG never shows or takes the
RAW file's rating; if the JPEG needs a sidecar of its own, it gets `IMG_0001.JPG.xmp`. The
same goes for the photo and video halves of an iPhone Live Photo.

When Frameloft edits an existing sidecar it changes only the tags you edited. Lightroom's
develop settings and everything else in the file stay as they were.

## Edits made in other apps

Frameloft watches your library folders. When Lightroom (Metadata › Save Metadata to File, or
with "Automatically write changes into XMP" switched on), Capture One or another app
updates a photo or its sidecar, Frameloft reads it again. A rating, label or keyword list in the
sidecar wins over the one the camera wrote into the RAW file, including when it was cleared.

Frameloft doesn't use Lightroom's pick and reject flags, which Lightroom keeps in its own
catalogue rather than in the files. A rejected rating (−1) written by Bridge shows as no stars.
