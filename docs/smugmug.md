# Uploading to SmugMug

Select photos, right-click, and choose **Upload to SmugMug…**.

## First time: get a SmugMug API key

SmugMug asks every app that uploads photos to use its own **API key**. Lumio is open
source, so a key built into the app would not stay private; instead, each person uses
their own. It is free for anyone with a SmugMug account and takes about two minutes:

1. In the upload window, click **SmugMug's API key page** (or open
   <https://api.smugmug.com/api/developer/apply>) and sign in to SmugMug.
2. Fill in the short form, for example:
   - Name: *Lumio (personal)*
   - Type: *Application*, Platform: *Desktop*, Use: *Personal*
   - Description: *Uploading my own photos from Lumio*
3. Accept the terms and submit. SmugMug shows a **Key** and a **Secret**. (You can
   look them up again later from "Manage Applications" on SmugMug's API pages.)
4. Paste both into Lumio and click **Save and continue**.

## Signing in

Click **Sign in to SmugMug**. Your browser opens a SmugMug page asking whether Lumio may
add photos to your account; click **Authorize**. SmugMug then shows a six-digit code.
Type it into Lumio and click **Finish signing in**.

You only do this once. The key, secret and sign-in are stored in your computer's
keychain (macOS Keychain, Windows Credential Manager, or the Secret Service on Linux),
not in the Lumio catalogue. To disconnect, click **Sign out** in the upload window, or
revoke Lumio's access in your SmugMug account settings (the list of apps you have
authorized).

Lumio asks only for permission to **add** photos and galleries; it can't change or
delete anything already on your SmugMug site.

## Uploading

- **Existing gallery** lists your galleries, most recently changed first.
- **New gallery** creates one at the top level of your site. *Unlisted* (anyone with
  the link) is the default; *Public* also shows it on your site; *Private* is only you.
- **Original files** sends JPEG, PNG, GIF, HEIC and video files exactly as they are.
  RAW, TIFF, WebP and other formats SmugMug can't display are converted to full-size,
  high-quality JPEGs (with their metadata) first.
- **Smaller JPEG copies** sends resized JPEGs (2048 to 6000 px on the long side), which
  is quicker on a slow connection. Videos still go as they are.

Title, caption and keywords from Lumio are sent with each photo. Uploads run one at a
time; you can cancel at any point, and photos already sent stay in the gallery. Lumio
does not check for duplicates, so uploading the same photo twice gives two copies.

## Troubleshooting

- *"SmugMug didn't accept the API key and secret"*: copy both again in full. Click
  **Use a different API key** to re-enter them.
- *"SmugMug didn't accept that code"*: codes work once and expire after a few minutes.
  Click **Back**, then **Sign in to SmugMug** for a new one.
- *"SmugMug no longer accepts Lumio's sign-in"*: you (or SmugMug) revoked access.
  Sign out and sign in again.
- Details of any failure are written to `lumio.log` in Lumio's data folder.

## For developers

`src-tauri/src/smugmug.rs` implements OAuth 1.0a (HMAC-SHA1, out-of-band PIN) against
`api.smugmug.com/services/oauth/1.0a/*`, lists albums via `/api/v2!authuser` →
`UserAlbums`, creates albums by POSTing `{"Type":"Album", …}` to the user's root node
`!children`, and uploads by streaming the file to `https://upload.smugmug.com/` with
`X-Smug-AlbumUri`, `X-Smug-Version: v2` and `X-Smug-ResponseType: JSON`.
