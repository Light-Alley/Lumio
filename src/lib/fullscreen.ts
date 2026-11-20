// Full screen for the viewer and the slideshow. Uses the window's own full screen
// (macOS gets its own Space; Windows and Linux cover the taskbar), and keeps
// `app.fullscreen` in step when the user leaves it another way, e.g. the green button.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { app } from "./store.svelte";

let watching = false;

async function sync() {
  try { app.fullscreen = await getCurrentWindow().isFullscreen(); } catch { /* not in Tauri */ }
}

/** Start following the window's full-screen state. Safe to call more than once. */
export function watchFullscreen() {
  if (watching) return;
  watching = true;
  sync();
  getCurrentWindow().onResized(sync).catch(() => {});
}

export async function setFullscreen(on: boolean) {
  watchFullscreen();
  try {
    await getCurrentWindow().setFullscreen(on);
    app.fullscreen = on;
  } catch {
    // Outside Tauri (e.g. a browser during development) fall back to the web API.
    try {
      if (on) await document.documentElement.requestFullscreen();
      else if (document.fullscreenElement) await document.exitFullscreen();
      app.fullscreen = on;
    } catch { /* not allowed */ }
  }
}

export function toggleFullscreen() {
  return setFullscreen(!app.fullscreen);
}
