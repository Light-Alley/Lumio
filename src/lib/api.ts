// Typed wrappers around the Rust commands.
import { invoke } from "@tauri-apps/api/core";

export interface Root { id: number; path: string; name: string; online: boolean; count: number }
export interface PhotoLite {
  id: number; path: string; filename: string; folder: string; ext: string; rating: number;
  label: string | null; taken_at: string | null; width: number | null; height: number | null;
  lat: number | null; lon: number | null; thumb: boolean; mtime: number; root_id: number;
  orientation: number | null;
  /** Photos grouped together share a stack id; `stack_top` marks the cover. */
  stack_id?: number | null; stack_top?: boolean; stack_count?: number;
  /** Videos only: length in seconds. */
  duration?: number | null;
}
export interface Photo extends PhotoLite {
  size: number; title: string | null; caption: string | null;
  keywords: string[]; creator: string | null; copyright: string | null; alt: number | null;
  place: string | null; make: string | null; model: string | null; lens: string | null;
  iso: number | null; fnumber: number | null; exposure: number | null; focal: number | null;
}
export interface FolderNode { path: string; name: string; count: number; has_children: boolean; online: boolean }
export interface OpReport { done: number; failed: [string, string][]; outside_library: boolean }
export interface AppInfo { name: string; id: string; is_default: boolean }
export interface Query {
  scope: string; folder?: string | null; recursive?: boolean; ids?: number[] | null; text?: string;
  min_rating?: number; label?: string; sort?: string; desc?: boolean;
}
export interface Edit {
  rating?: number; label?: string; title?: string; caption?: string; keywords?: string[];
  add_keywords?: string[]; remove_keywords?: string[]; creator?: string; copyright?: string;
  gps?: [number, number]; clear_gps?: boolean;
}
export interface AiSettings {
  provider: string; anthropic_model: string; effort: string; openai_base_url: string; openai_model: string;
  allow_images: boolean; has_anthropic_key: boolean; has_openai_key: boolean;
}
export interface ChatReply { text: string; result_ids: number[] | null; result_title: string | null }
export interface AppStatus { version: string; exiftool: boolean; exiftool_path: string; data_dir: string; photo_count: number }
export interface Place { name: string; lat: number; lon: number }

export type Turn = "left" | "right" | "flip_h" | "flip_v";

export const api = {
  getRoots: () => invoke<Root[]>("get_roots"),
  addRoot: (path: string) => invoke<number>("add_root", { path }),
  removeRoot: (id: number) => invoke<void>("remove_root", { id }),
  rescan: (rootId?: number, folder?: string) => invoke<void>("rescan", { rootId: rootId ?? null, folder: folder ?? null }),
  refreshPhotos: (ids: number[]) => invoke<void>("refresh_photos", { ids }),
  folderChildren: (path: string) => invoke<FolderNode[]>("folder_children", { path }),
  createFolder: (parent: string, name: string) => invoke<string>("create_folder", { parent, name }),
  renameFolder: (path: string, name: string) => invoke<string>("rename_folder", { path, name }),
  deleteFolder: (path: string, permanent: boolean) => invoke<void>("delete_folder", { path, permanent }),
  moveFolder: (src: string, dest: string, copy: boolean) => invoke<OpReport>("move_folder", { src, dest, copy }),
  queryPhotos: (q: Query) => invoke<PhotoLite[]>("query_photos", { q }),
  getPhoto: (id: number) => invoke<Photo | null>("get_photo", { id }),
  getPhotos: (ids: number[]) => invoke<Photo[]>("get_photos", { ids }),
  movePhotos: (ids: number[], dest: string, copy: boolean) => invoke<OpReport>("move_photos", { ids, dest, copy }),
  deletePhotos: (ids: number[], permanent: boolean) => invoke<OpReport>("delete_photos", { ids, permanent }),
  renamePhoto: (id: number, name: string) => invoke<void>("rename_photo", { id, name }),
  updateMetadata: (ids: number[], edit: Edit) => invoke<OpReport>("update_metadata", { ids, edit }),
  rotatePhotos: (ids: number[], op: Turn) => invoke<OpReport>("rotate_photos", { ids, op }),
  saveVideoThumb: (id: number, jpeg: string, width: number | null, height: number | null, duration: number | null) =>
    invoke<void>("save_video_thumb", { id, jpeg, width, height, duration }),
  stackPhotos: (ids: number[], top: number | null) => invoke<number>("stack_photos", { ids, top }),
  unstackPhotos: (ids: number[], whole: boolean) => invoke<void>("unstack_photos", { ids, whole }),
  setStackTop: (id: number) => invoke<void>("set_stack_top", { id }),
  listOpenWith: (id: number) => invoke<AppInfo[]>("list_open_with", { id }),
  openWith: (ids: number[], app: string) => invoke<void>("open_with", { ids, app }),
  openDefault: (ids: number[]) => invoke<void>("open_default", { ids }),
  reveal: (id?: number, path?: string) => invoke<void>("reveal", { id: id ?? null, path: path ?? null }),
  getSettings: () => invoke<Record<string, string>>("get_settings"),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
  appStatus: () => invoke<AppStatus>("app_status"),
  aiSettings: () => invoke<AiSettings>("ai_settings"),
  aiSetKey: (which: string, key: string) => invoke<void>("ai_set_key", { which, key }),
  aiChat: (text: string) => invoke<ChatReply>("ai_chat", { text }),
  aiReset: () => invoke<void>("ai_reset"),
  geocode: (query: string) => invoke<Place[]>("geocode", { query }),
};

export const VIDEO_EXTS = ["mp4", "m4v", "mov", "webm", "mkv", "avi", "3gp", "mts", "m2ts", "mpg", "mpeg", "wmv", "ogv"];
export const isVideo = (p: { ext: string } | null | undefined) => !!p && VIDEO_EXTS.includes(p.ext.toLowerCase());
/** 75 -> "1:15", 3725 -> "1:02:05". */
export function formatDuration(s: number | null | undefined): string {
  if (s == null || !isFinite(s) || s < 0) return "";
  const t = Math.floor(s);
  const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), sec = t % 60;
  const ss = String(sec).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

const isWindows = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);
export const isLinux = typeof navigator !== "undefined" && /Linux/i.test(navigator.userAgent) && !isWindows;
export const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
export const fileManagerName = isMac ? "Finder" : isWindows ? "File Explorer" : "file manager";

function scheme(name: string) {
  return isWindows ? `http://${name}.localhost` : `${name}://localhost`;
}
type Versioned = { id: number; mtime: number; orientation?: number | null };
export const thumbUrl = (p: Versioned) => `${scheme("thumb")}/${p.id}?m=${p.mtime}-${p.orientation ?? 1}`;
export const photoUrl = (p: Versioned) => `${scheme("photo")}/${p.id}?m=${p.mtime}-${p.orientation ?? 1}`;

export function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as Error).message);
  return String(e);
}

export const LABELS = [
  { name: "Red", color: "#e5484d" },
  { name: "Yellow", color: "#f5d90a" },
  { name: "Green", color: "#46a758" },
  { name: "Blue", color: "#3e63dd" },
  { name: "Purple", color: "#8e4ec6" },
];
export const labelColor = (l: string | null | undefined) => LABELS.find((x) => x.name.toLowerCase() === (l ?? "").toLowerCase())?.color;

export function formatDate(s: string | null | undefined): string {
  if (!s) return "";
  const d = new Date(s);
  if (isNaN(d.getTime())) return s;
  return d.toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB"];
  let i = -1;
  do { n /= 1024; i++; } while (n >= 1024 && i < u.length - 1);
  return `${n.toFixed(n < 10 ? 1 : 0)} ${u[i]}`;
}
export function formatExposure(x: number | null): string {
  if (!x) return "";
  return x >= 1 ? `${x}s` : `1/${Math.round(1 / x)}s`;
}
export function basename(p: string) {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}
export function parentOf(p: string) {
  const parts = p.split(/([\\/])/);
  // drop last segment and its separator
  let i = parts.length - 1;
  while (i > 0 && (parts[i] === "" || parts[i] === "/" || parts[i] === "\\")) i--;
  return parts.slice(0, Math.max(i - 1, 1)).join("");
}

// ------------------------------------------------------------ sharing
export interface ShareOutcome { message: string }
export interface ShareOptions { sheet: boolean; sites: { key: string; name: string }[] }
export const shareApi = {
  options: () => invoke<ShareOptions>("share_options"),
  sheet: (ids: number[], x: number, y: number) => invoke<ShareOutcome>("share_sheet", { ids, x, y }),
  email: (ids: number[]) => invoke<ShareOutcome>("share_email", { ids }),
  whatsapp: (ids: number[], web: boolean) => invoke<ShareOutcome>("share_whatsapp", { ids, web }),
  site: (ids: number[], site: string) => invoke<ShareOutcome>("share_site", { ids, site }),
  copy: (ids: number[]) => invoke<ShareOutcome>("share_copy", { ids }),
  show: (ids: number[]) => invoke<ShareOutcome>("share_show", { ids }),
};
