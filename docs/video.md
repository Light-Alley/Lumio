# Video preview

Lumio lists videos alongside photos and can play them with a small built-in player
(play/pause, back and forward 10 seconds, a position bar and volume). It is meant for a
quick look, not as a full media player.

## Why it stays light

Lumio does not bundle a video decoder (such as FFmpeg or VLC's libVLC, which would add
50–100 MB to every download). It plays videos with the HTML5 player of the web view the
app already runs in, which uses the operating system's own video support:

| System | Web view | Video support comes from |
|---|---|---|
| macOS | WKWebView (Safari) | AVFoundation, built in |
| Windows 10/11 | WebView2 (Microsoft Edge) | Media Foundation, built in |
| Linux | WebKitGTK | GStreamer plugins installed on the system |

The video file is streamed in small pieces, so starting and seeking are quick even for
long videos on network drives, and the file is never loaded into memory whole.

On macOS and Windows the player reads videos from Lumio's own `photo://` address. On
Linux, WebKitGTK refuses to play media from such custom addresses, so there Lumio starts
a tiny web server that only listens on the computer itself (`127.0.0.1`, a free port)
and only answers requests carrying a secret that is made fresh each time Lumio starts.
Other programs and web pages can't read files through it. See
`src-tauri/src/videoserver.rs`.

## What plays where

| Format | macOS | Windows | Linux (with the plugins below) |
|---|---|---|---|
| MP4 / M4V / MOV, H.264 | ✓ | ✓ | ✓ |
| MP4 / MOV, HEVC (H.265, e.g. iPhone) | ✓ | only with the free *HEVC Video Extensions* from the Microsoft Store and a supported graphics card | ✓ with gst-libav |
| WebM (VP8 / VP9 / AV1) | ✓ (recent macOS) | ✓ | ✓ |
| MKV | ✗ | often, if the codecs inside are H.264 / VP9 | ✓ |
| AVI, WMV, MPG, MTS / M2TS, 3GP | ✗ | mostly ✗ | mostly ✓ with gst-libav |
| ProRes | ✓ | ✗ | ✗ |

When a video can't be played on the current computer, Lumio says so and offers
**Open in video player**, which opens the file in the system's default app (for example
VLC, if that is set as the default).

## Linux: GStreamer plugins

WebKitGTK plays video through GStreamer. Most desktop installs already include enough to
play MP4 and WebM. For the widest support install:

- **Ubuntu / Debian**: `sudo apt install gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav`
- **Fedora**: `sudo dnf install gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-libav`
  (H.264 / HEVC may need the RPM Fusion versions)
- **Arch**: `sudo pacman -S gst-plugins-good gst-plugins-bad gst-libav`

## Thumbnails

- A preview picture saved inside the video by the camera is used when there is one.
- **macOS**: otherwise Lumio asks Quick Look for a thumbnail, like Finder shows.
- **Windows and Linux** (and anything Quick Look can't do): Lumio opens the video in its
  own player in the background, takes a frame about one second in, and saves that as the
  thumbnail. Videos the player can't open keep a film icon.

## Metadata

Ratings, labels, titles, keywords and locations are written as XMP. MP4, M4V, MOV and 3GP
files hold XMP themselves; for other video formats Lumio writes an XMP sidecar next to the
file (as it does for RAW photos). Videos can't be rotated in Lumio.
