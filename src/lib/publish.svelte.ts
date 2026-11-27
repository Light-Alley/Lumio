// Getting photos out of Frameloft: SmugMug uploads and web-ready copies (e.g. for Squarespace).
// Kept apart from the main store so these features stay easy to find and change.
import { invoke } from "@tauri-apps/api/core";

export interface SmugStatus { has_app_key: boolean; connected: boolean; nickname: string; apply_url: string }
export interface SmugAlbum {
  name: string; uri: string; web_uri: string; path: string; privacy: string;
  image_count: number | null; updated: string;
}
export interface SmugUploadOptions { size: "original" | "web"; long_edge: number | null }
export interface SmugReport { done: number; failed: [string, string][]; cancelled: boolean; web_uri: string }
export interface SmugProgress { done: number; total: number; current: string; bytes: number; size: number }

export interface WebExportOptions { long_edge: number | null; quality: number; strip_location: boolean }
export interface WebExportReport { done: number; failed: [string, string][]; skipped_videos: number; cancelled: boolean; folder: string }
export interface WebExportProgress { done: number; total: number; current: string }

export const smugmug = {
  status: () => invoke<SmugStatus>("smugmug_status"),
  setAppKey: (key: string, secret: string) => invoke<SmugStatus>("smugmug_set_app_key", { key, secret }),
  beginSignIn: () => invoke<string>("smugmug_begin_sign_in"),
  finishSignIn: (code: string) => invoke<SmugStatus>("smugmug_finish_sign_in", { code }),
  signOut: (forgetKey: boolean) => invoke<SmugStatus>("smugmug_sign_out", { forgetKey }),
  albums: () => invoke<SmugAlbum[]>("smugmug_albums"),
  createAlbum: (name: string, privacy: string) => invoke<SmugAlbum>("smugmug_create_album", { name, privacy }),
  upload: (ids: number[], albumUri: string, webUri: string, opts: SmugUploadOptions) =>
    invoke<SmugReport>("smugmug_upload", { ids, albumUri, webUri, opts }),
  cancel: () => invoke<void>("smugmug_cancel"),
  open: (url: string) => invoke<void>("smugmug_open", { url }),
};

export const webExport = {
  run: (ids: number[], folder: string, opts: WebExportOptions) => invoke<WebExportReport>("web_export", { ids, folder, opts }),
  cancel: () => invoke<void>("web_export_cancel"),
};

/** Which photos each window is working on; `null` when the window is closed. */
export const publish = $state({
  smugmugIds: null as number[] | null,
  exportIds: null as number[] | null,
});

export function openSmugMug(ids: number[]) {
  if (ids.length) publish.smugmugIds = [...ids];
}
export function openWebExport(ids: number[]) {
  if (ids.length) publish.exportIds = [...ids];
}

/** Small remembered choices (last gallery, export size). Losing them is harmless. */
export function remember<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(`lumio.${key}`);
    return v == null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
}
export function keep(key: string, value: unknown) {
  try { localStorage.setItem(`lumio.${key}`, JSON.stringify(value)); } catch { /* ignore */ }
}
