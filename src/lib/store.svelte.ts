// Application state (Svelte 5 runes) and the actions the UI performs.
import { SvelteSet } from "svelte/reactivity";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, errText, type PhotoLite, type Root, type Edit, type OpReport } from "./api";

export type Scope =
  | { kind: "all" }
  | { kind: "recent" }
  | { kind: "rated" }
  | { kind: "geotagged" }
  | { kind: "nogps" }
  | { kind: "folder"; path: string; name: string; rootId?: number }
  | { kind: "ai"; ids: number[]; title: string };

export type Bg = "black" | "grey" | "lightgrey" | "white";

interface Toast { id: number; text: string; kind: "info" | "error" | "success"; action?: { label: string; run: () => void } }
interface DialogState {
  kind: "prompt" | "confirm";
  title: string;
  message?: string;
  value?: string;
  okLabel?: string;
  danger?: boolean;
  extra?: { label: string; value: string };
  resolve: (v: string | boolean | null) => void;
}

export const app = $state({
  roots: [] as Root[],
  scope: { kind: "all" } as Scope,
  recursive: false,
  photos: [] as PhotoLite[],
  loading: false,
  text: "",
  minRating: 0,
  label: "",
  sort: "date",
  desc: true,
  thumbSize: 180,
  view: "grid" as "grid" | "map",
  panel: "info" as "info" | "ai" | null,
  viewerIndex: -1,
  viewerBg: "black" as Bg,
  theme: "dark" as "dark" | "light",
  focusId: null as number | null,
  anchorId: null as number | null,
  progress: { active: false, done: 0, total: 0, message: "" },
  op: { done: 0, total: 0, message: "" },
  toasts: [] as Toast[],
  dialog: null as DialogState | null,
  showSettings: false,
  folderVersion: 0,
  settings: {} as Record<string, string>,
  exiftool: true,
  dragIds: [] as number[],
});

export const selection = new SvelteSet<number>();

export function selectedIds(): number[] {
  return app.photos.filter((p) => selection.has(p.id)).map((p) => p.id);
}

export function rootOnline(rootId: number): boolean {
  return app.roots.find((r) => r.id === rootId)?.online ?? false;
}

// ------------------------------------------------------------------ toasts & dialogs

let toastSeq = 1;
export function toast(text: string, kind: Toast["kind"] = "info", action?: Toast["action"]) {
  const id = toastSeq++;
  app.toasts.push({ id, text, kind, action });
  setTimeout(() => dismissToast(id), kind === "error" ? 9000 : action ? 8000 : 3500);
}
export function dismissToast(id: number) {
  const i = app.toasts.findIndex((t) => t.id === id);
  if (i >= 0) app.toasts.splice(i, 1);
}
export function errorToast(e: unknown) {
  toast(errText(e), "error");
}

export function prompt(title: string, value = "", okLabel = "OK", message?: string): Promise<string | null> {
  return new Promise((resolve) => {
    app.dialog = { kind: "prompt", title, value, okLabel, message, resolve: (v) => resolve(v as string | null) };
  });
}
export function confirm(title: string, message: string, okLabel = "OK", danger = false): Promise<boolean> {
  return new Promise((resolve) => {
    app.dialog = { kind: "confirm", title, message, okLabel, danger, resolve: (v) => resolve(!!v) };
  });
}

function reportResult(r: OpReport, verb: string) {
  if (r.failed.length) {
    const first = r.failed.slice(0, 3).map(([f, e]) => `${f}: ${e}`).join("\n");
    toast(`${r.done} ${verb}, ${r.failed.length} failed.\n${first}`, "error");
  } else if (r.done) {
    toast(`${r.done} photo${r.done === 1 ? "" : "s"} ${verb}.`, "success");
  }
}

// ------------------------------------------------------------------ loading

export async function loadRoots() {
  app.roots = await api.getRoots();
}

let loadSeq = 0;
export async function loadPhotos() {
  const seq = ++loadSeq;
  app.loading = true;
  const s = app.scope;
  try {
    const photos = await api.queryPhotos({
      scope: s.kind === "ai" ? "ids" : s.kind,
      folder: s.kind === "folder" ? s.path : null,
      recursive: app.recursive,
      ids: s.kind === "ai" ? s.ids : null,
      text: app.text,
      min_rating: app.minRating,
      label: app.label,
      sort: s.kind === "ai" && app.sort === "relevance" ? undefined : app.sort,
      desc: app.desc,
    });
    if (seq !== loadSeq) return;
    app.photos = photos;
    const ids = new Set(photos.map((p) => p.id));
    for (const id of [...selection]) if (!ids.has(id)) selection.delete(id);
    if (app.viewerIndex >= photos.length) app.viewerIndex = photos.length - 1;
  } catch (e) {
    errorToast(e);
  } finally {
    if (seq === loadSeq) app.loading = false;
  }
}

export function setScope(s: Scope) {
  app.scope = s;
  selection.clear();
  app.focusId = null;
  app.anchorId = null;
  if (s.kind === "ai") app.sort = "relevance";
  else if (app.sort === "relevance") app.sort = "date";
  loadPhotos();
}

let refreshTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleRefresh() {
  if (refreshTimer) return;
  refreshTimer = setTimeout(async () => {
    refreshTimer = null;
    await loadRoots();
    await loadPhotos();
    app.folderVersion++;
  }, 600);
}

export async function init() {
  try {
    app.settings = await api.getSettings();
    const st = app.settings;
    if (st.viewer_bg) app.viewerBg = st.viewer_bg as Bg;
    if (st.theme) app.theme = st.theme as "dark" | "light";
    if (st.thumb_size) app.thumbSize = Number(st.thumb_size) || 180;
    if (st.recursive) app.recursive = st.recursive === "1";
    const status = await api.appStatus();
    app.exiftool = status.exiftool;
    await loadRoots();
    await loadPhotos();
  } catch (e) {
    errorToast(e);
  }
  await listen("library-changed", scheduleRefresh);
  await listen<typeof app.progress>("index-progress", (e) => (app.progress = e.payload));
  await listen<typeof app.op>("op-progress", (e) => (app.op = e.payload));
  // When the window regains focus, pick up drives that were (dis)connected and, at most
  // every 10 minutes, rescan for changes the file watcher can't see (e.g. network drives).
  let lastRescan = Date.now();
  window.addEventListener("focus", async () => {
    const before = app.roots.map((r) => r.online).join();
    await loadRoots();
    const changed = before !== app.roots.map((r) => r.online).join();
    if (changed || Date.now() - lastRescan > 10 * 60 * 1000) {
      lastRescan = Date.now();
      api.rescan().catch(() => {});
      if (changed) { loadPhotos(); app.folderVersion++; }
    }
  });
}

export function saveSetting(key: string, value: string) {
  app.settings[key] = value;
  api.setSetting(key, value).catch(errorToast);
}

// ------------------------------------------------------------------ library actions

export async function addFolderToLibrary(path?: string) {
  const p = path ?? ((await openDialog({ directory: true, multiple: false, title: "Add a folder to your library" })) as string | null);
  if (!p) return;
  try {
    await api.addRoot(p);
    await loadRoots();
    toast("Folder added. Building thumbnails in the background…", "success");
  } catch (e) {
    errorToast(e);
  }
}

export async function removeRoot(root: Root) {
  const ok = await confirm(
    `Remove “${root.name}” from the library?`,
    "Lumio forgets this folder and its thumbnails. Your photos stay where they are on disk.",
    "Remove",
  );
  if (!ok) return;
  try {
    await api.removeRoot(root.id);
    if (app.scope.kind === "folder" && app.scope.rootId === root.id) setScope({ kind: "all" });
    await loadRoots();
    await loadPhotos();
    app.folderVersion++;
  } catch (e) {
    errorToast(e);
  }
}

// ------------------------------------------------------------------ metadata actions

export async function applyEdit(ids: number[], edit: Edit, quiet = false) {
  if (!ids.length) return;
  // Optimistic update for rating/label so the grid reacts immediately.
  for (const p of app.photos) {
    if (!ids.includes(p.id)) continue;
    if (edit.rating !== undefined) p.rating = edit.rating;
    if (edit.label !== undefined) p.label = edit.label || null;
    if (edit.gps) { p.lat = edit.gps[0]; p.lon = edit.gps[1]; }
    if (edit.clear_gps) { p.lat = null; p.lon = null; }
  }
  try {
    const r = await api.updateMetadata(ids, edit);
    if (r.failed.length || !quiet) reportResult(r, "updated");
    await loadPhotos();
    app.folderVersion++;
  } catch (e) {
    errorToast(e);
    loadPhotos();
  }
}

export function rate(ids: number[], rating: number) {
  return applyEdit(ids, { rating }, true);
}
export function setLabel(ids: number[], label: string) {
  return applyEdit(ids, { label }, true);
}

// ------------------------------------------------------------------ file actions

export async function moveOrCopy(ids: number[], copy: boolean, dest?: string) {
  if (!ids.length) return;
  const d = dest ?? ((await openDialog({ directory: true, multiple: false, title: copy ? "Copy photos to…" : "Move photos to…" })) as string | null);
  if (!d) return;
  try {
    const r = await api.movePhotos(ids, d, copy);
    reportResult(r, copy ? "copied" : "moved");
    if (r.outside_library && r.done) {
      toast(`That folder isn't in your library${copy ? "" : ", so the moved photos left the catalogue"}.`, "info", {
        label: "Add folder to library",
        run: () => addFolderToLibrary(d),
      });
    }
    await loadPhotos();
    app.folderVersion++;
  } catch (e) {
    errorToast(e);
  }
}

export async function trashPhotos(ids: number[]) {
  if (!ids.length) return;
  const n = ids.length;
  const ok = await confirm(
    `Move ${n} photo${n === 1 ? "" : "s"} to the Trash?`,
    "Their XMP sidecar files go too. You can restore them from the system Trash / Recycle Bin.",
    "Move to Trash",
    true,
  );
  if (!ok) return;
  try {
    let r = await api.deletePhotos(ids, false);
    if (r.failed.length && r.failed.every(([, e]) => /Trash/i.test(e))) {
      const again = await confirm(
        "This drive has no Trash",
        "Network and some external drives can't use the Trash. Delete these photos permanently instead? This cannot be undone.",
        "Delete permanently",
        true,
      );
      if (again) {
        const failedNames = new Set(r.failed.map(([f]) => f));
        const retry = app.photos.filter((p) => ids.includes(p.id) && failedNames.has(p.filename)).map((p) => p.id);
        const r2 = await api.deletePhotos(retry, true);
        r = { done: r.done + r2.done, failed: r2.failed, outside_library: false };
      }
    }
    reportResult(r, "deleted");
    selection.clear();
    await loadPhotos();
    app.folderVersion++;
  } catch (e) {
    errorToast(e);
  }
}

export async function renamePhoto(p: PhotoLite) {
  const name = await prompt("Rename photo", p.filename, "Rename");
  if (!name || name === p.filename) return;
  try {
    await api.renamePhoto(p.id, name);
    await loadPhotos();
  } catch (e) {
    errorToast(e);
  }
}

export async function newFolder(parent: string) {
  const name = await prompt("New folder", "", "Create", `Inside ${parent}`);
  if (!name) return;
  try {
    await api.createFolder(parent, name);
    app.folderVersion++;
  } catch (e) {
    errorToast(e);
  }
}

export async function renameFolder(path: string, current: string) {
  const name = await prompt("Rename folder", current, "Rename");
  if (!name || name === current) return;
  try {
    const np = await api.renameFolder(path, name);
    if (app.scope.kind === "folder" && app.scope.path === path) app.scope = { ...app.scope, path: np, name };
    await loadRoots();
    await loadPhotos();
    app.folderVersion++;
  } catch (e) {
    errorToast(e);
  }
}

export async function trashFolder(path: string, name: string) {
  const ok = await confirm(
    `Move the folder “${name}” to the Trash?`,
    "Everything inside it, including photos, goes to the system Trash / Recycle Bin.",
    "Move to Trash",
    true,
  );
  if (!ok) return;
  try {
    await api.deleteFolder(path, false);
  } catch (e) {
    const perm = await confirm(
      "This drive has no Trash",
      `${errText(e)}\n\nDelete the folder permanently instead? This cannot be undone.`,
      "Delete permanently",
      true,
    );
    if (!perm) return;
    try {
      await api.deleteFolder(path, true);
    } catch (e2) {
      return errorToast(e2);
    }
  }
  if (app.scope.kind === "folder" && app.scope.path.startsWith(path)) setScope({ kind: "all" });
  toast(`“${name}” moved to the Trash.`, "success");
  app.folderVersion++;
}

export async function dropOnFolder(dest: string, e: DragEvent) {
  const copy = e.altKey;
  const photoData = e.dataTransfer?.getData("application/x-lumio-photos");
  const folderData = e.dataTransfer?.getData("application/x-lumio-folder");
  if (photoData) {
    const ids = JSON.parse(photoData) as number[];
    await moveOrCopy(ids, copy, dest);
  } else if (folderData) {
    if (folderData === dest || dest.startsWith(folderData)) return;
    try {
      const r = await api.moveFolder(folderData, dest, copy);
      toast(`Folder ${copy ? "copied" : "moved"}.`, "success");
      if (r.outside_library) toast("The destination isn't in your library.", "info");
      await loadPhotos();
      app.folderVersion++;
    } catch (err) {
      errorToast(err);
    }
  }
}

// ------------------------------------------------------------------ selection

export function selectOnly(id: number) {
  selection.clear();
  selection.add(id);
  app.focusId = id;
  app.anchorId = id;
}

export function clickSelect(id: number, e: MouseEvent | KeyboardEvent) {
  const meta = e.metaKey || e.ctrlKey;
  if (e.shiftKey && app.anchorId !== null) {
    const ids = app.photos.map((p) => p.id);
    const a = ids.indexOf(app.anchorId);
    const b = ids.indexOf(id);
    if (a >= 0 && b >= 0) {
      if (!meta) selection.clear();
      for (let i = Math.min(a, b); i <= Math.max(a, b); i++) selection.add(ids[i]);
    }
    app.focusId = id;
  } else if (meta) {
    if (selection.has(id)) selection.delete(id);
    else selection.add(id);
    app.focusId = id;
    app.anchorId = id;
  } else {
    selectOnly(id);
  }
}

export function openViewer(id: number) {
  const i = app.photos.findIndex((p) => p.id === id);
  if (i >= 0) app.viewerIndex = i;
}
