// Thumbnails for videos the system couldn't make one for (Windows and Linux have no
// thumbnailer Lumio can call without bundling a video decoder). The webview's own
// player opens the video, jumps a little way in, and the frame is saved as the
// thumbnail. One video at a time, in the background; videos the player can't open
// keep the film icon.
import { untrack } from "svelte";
import { api, isVideo, photoUrl, type PhotoLite } from "./api";
import { app, rootOnline } from "./store.svelte";

const tried = new Set<number>();
const queue: PhotoLite[] = [];
let running = false;

export function makeMissingVideoThumbs(photos: PhotoLite[]) {
  for (const p of photos) {
    if (isVideo(p) && !p.thumb && !tried.has(p.id) && rootOnline(p.root_id)) {
      tried.add(p.id);
      queue.push(p);
    }
  }
  if (!running && queue.length) void run();
}

async function run() {
  running = true;
  while (queue.length) {
    const p = queue.shift()!;
    try { await capture(p); } catch (e) { console.warn("video thumbnail", p.filename, e); }
  }
  running = false;
}

function capture(p: PhotoLite): Promise<void> {
  return new Promise((resolve) => {
    const v = document.createElement("video");
    v.muted = true;
    v.preload = "auto";
    v.playsInline = true;
    // The video comes from Lumio's own photo:// address; without this the frame
    // could not be read back from the canvas.
    v.crossOrigin = "anonymous";
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      v.removeAttribute("src");
      v.load();
      resolve();
    };
    const timer = setTimeout(finish, 20000);
    v.onerror = finish;
    v.onloadedmetadata = () => {
      const d = v.duration;
      v.currentTime = isFinite(d) && d > 0 ? Math.min(1, d / 10) : 0;
    };
    v.onseeked = async () => {
      try {
        if (!v.videoWidth || !v.videoHeight) return;
        const scale = Math.min(1, 480 / Math.max(v.videoWidth, v.videoHeight));
        const c = document.createElement("canvas");
        c.width = Math.max(1, Math.round(v.videoWidth * scale));
        c.height = Math.max(1, Math.round(v.videoHeight * scale));
        c.getContext("2d")!.drawImage(v, 0, 0, c.width, c.height);
        const url = c.toDataURL("image/jpeg", 0.85);
        const duration = isFinite(v.duration) && v.duration > 0 ? v.duration : null;
        await api.saveVideoThumb(p.id, url.slice(url.indexOf(",") + 1), v.videoWidth, v.videoHeight, duration);
        for (const list of [app.photos, app.allPhotos]) {
          for (const x of list) {
            if (x.id === p.id) { x.thumb = true; x.duration ??= duration; }
          }
        }
      } catch (e) {
        console.warn("video thumbnail", p.filename, e);
      } finally {
        finish();
      }
    };
    v.src = photoUrl(p);
  });
}

/** Call from a component: keep up with the photo list as it changes. */
export function watchVideoThumbs() {
  $effect(() => {
    const list = app.photos;
    app.roots; // drives coming online
    untrack(() => makeMissingVideoThumbs(list));
  });
}
