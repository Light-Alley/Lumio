# Smart collections

A smart collection is a saved search. It stores **rules**, not photos, so it is always
up to date: a photo joins as soon as it matches (you rate it 5 stars, add a keyword,
move it into the folder) and leaves when it stops matching. Nothing is copied or moved,
and deleting a collection never touches your photos.

Make one with the **+** next to *Smart collections* in the sidebar, or right-click a
folder and choose **New smart collection from this folder…**. Right-click a collection
to edit, duplicate, delete, open it in a new tab or play it as a slideshow. The search
box, star and colour filters in the toolbar still work on top of a collection.

## Rules

A collection matches photos that meet **all** of its rules, or **any** of them.

| Rule | Conditions |
|---|---|
| Rating | at least, exactly, at most (0 to 5 stars) |
| Date taken | in a year, in a month (any year), between two dates, on or after, on or before, in the last N days, unknown. Photos without a date taken (many PNGs and videos) use the file's date, as the grid does. |
| Colour label | is, is not, has any, has none |
| Keyword | includes, doesn't include (whole keywords: "Italy" doesn't match "Italy food"), has any, has none |
| Folder | is in (with its subfolders), is not in |
| Camera | contains, doesn't contain (make and model together, for example "Canon EOS R5"), unknown |
| Lens | contains, unknown |
| File type | is, is not: photo (anything but video), RAW, JPEG, HEIC/AVIF, PNG, TIFF, video |
| Any text | contains, doesn't contain (name, title, caption, keywords and place) |
| File name | contains, starts with, doesn't contain |
| Location | is on the map, is missing; Place name contains |
| Shape | landscape, portrait, square (as the photo is shown, after rotation) |
| Added to Frameloft | in the last N days |
| ISO, focal length, aperture | at least, at most |

Rules that are still empty (no value typed yet) are left out, and a collection with no
complete rules shows every photo.

## How it works

Collections live in the catalogue's `collections` table as JSON:

```json
{ "any": false, "rules": [
  { "field": "rating", "op": "gte", "value": "5", "value2": "" },
  { "field": "taken",  "op": "year", "value": "2024", "value2": "" }
] }
```

`src-tauri/src/collections.rs` turns each rule into an SQL condition (`rule_sql`) with
its values passed as parameters, never pasted into the SQL. Unknown rules are refused
when saving, and read as "no match" if a newer version of Frameloft wrote one.

### Adding a rule

1. Add a match arm to `rule_sql` in `src-tauri/src/collections.rs`, for example
   `("flag", "is") => format!("flag = {}", arg(args, v.into()))`, and a test.
2. Add the field and its conditions to `FIELDS` in `src/lib/collections.ts`. The value
   kinds are `number`, `text`, `date` (with `between` for two dates), `choice` (a list),
   `stars`, `folder` and `none`.

That's all: the editor, the live count and the sidebar pick it up.
