// Application state (Svelte 5 runes) and the actions the UI performs.
import { SvelteSet } from "svelte/reactivity";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, errText, type PhotoLite, type Root, type Edit, type OpReport, type Turn, type PendingMove, type PendingReport, type HistoryEntry, type Collection, type RuleSet } from "./api";

export type Scope =
  | { kind: "all" }
  | { kind: "recent" }
  | { kind: "rated" }
  | { kind: "geotagged" }
  | { kind: "nogps" }
  | { kind: "folder"; path: string; name: string; rootId?: number }
  | { kind: "collection"; id: number; name: string }
  | { kind: "ai"; ids: number[]; title: string };

export type Bg = "black" | "grey" | "lightgrey" | "white";
export type View = "grid" | "list" | "filmstrip" | "map";
export type Panel = "info" | "ai" | null;
export const SIDEBAR_DEFAULT = 250;
export const SIDEBAR_MIN = 180;
export const SIDEBAR_MAX = 480;

/** What each tab remembers: where it's looking and how. */
export interface Tab {
  id: number;
  scope: Scope;
  recursive: boolean;
  text: string;
  minRating: number;
  label: string;
  sort: string;
  desc: boolean;
  view: View;
  focusId: number | null;
}

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
  /** What's shown: `allPhotos` with closed stacks reduced to their cover. */
  photos: [] as PhotoLite[],
  /** Everything the current query returned. */
  allPhotos: [] as PhotoLite[],
  loading: false,
  text: "",
  minRating: 0,
  label: "",
  sort: "date",
  desc: true,
  thumbSize: 180,
  view: "grid" as View,
  panel: "info" as Panel,
  /** Width of the folder sidebar in pixels (dragged by its right edge, remembered). */
  sidebarWidth: SIDEBAR_DEFAULT,
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
  /** Smart collections (saved rules), with how many photos each matches. */
  collections: [] as Collection[],
  /** The collection being edited (`id` null for a new one); `null` when the editor is closed. */
  collectionEdit: null as { id: number | null; name: string; rules: RuleSet } | null,
  /** The duplicate finder, and the folder it looks in (`null` = whole library); `null` when closed. */
  dupes: null as { folder: string | null; name: string } | null,
  /** Photos in the batch rename window; `null` when it is closed. */
  renameIds: null as number[] | null,
  /** The running slideshow; `null` when none is playing. */
  slideshow: null as { photos: PhotoLite[]; title: string; start: number; backToViewer?: boolean } | null,
  /** The window fills the screen (set by `fullscreen.ts`). */
  fullscreen: false,
  folderVersion: 0,
  settings: {} as Record<string, string>,
  exiftool: true,
  dragIds: [] as number[],
  /** Photos being dragged. Their rows stay mounted (hidden) if a tab switches mid-drag, because removing a drag's source element cancels the drag. */
  dragPhotos: [] as PhotoLite[],
  tabs: [] as Tab[],
  /** Moves waiting for a drive to be connected. */
  pending: [] as PendingMove[],
  showPending: false,
  /** The undo history, newest first. */
  history: [] as HistoryEntry[],
  showHistory: false,
  /** An undo is running. */
  undoing: false,
  activeTab: 0,
});

export const selection = new SvelteSet<number>();

// ------------------------------------------------------------------ side panels

/** Show the info or AI panel on the right, or hide it (`null`). Remembered between sessions. */
export function setPanel(p: Panel) {
  app.panel = p;
  saveSetting("panel", p ?? "none");
}

/** Open the panel, or hide it if it is already showing. */
export function togglePanel(p: "info" | "ai" = "info") {
  setPanel(app.panel === p ? null : p);
}

/** Keeps the folder sidebar between a usable minimum and a sensible share of the window. */
export function clampSidebar(w: number): number {
  const max = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, Math.round(window.innerWidth * 0.4)));
  return Math.round(Math.max(SIDEBAR_MIN, Math.min(max, w)));
}
/** Stacks the user has opened to see every photo in them. */
export const openStacks = new SvelteSet<number>();

export function selectedIds(): number[] {
  return app.photos.filter((p) => selection.has(p.id)).map((p) => p.id);
}

export function rootOnline(rootId: number): boolean {
  return app.roots.find((r) => r.id === rootId)?.online ?? false;
}

// ------------------------------------------------------------------ toasts & dialogs

let toastSeq = 1;
export function toast(text: string, kind: Toast["kind"] = "info", action?: Toast["action"], ms?: number) {
  const id = toastSeq++;
  app.toasts.push({ id, text, kind, action });
  setTimeout(() => dismissToast(id), ms ?? (kind === "error" ? 9000 : action ? 8000 : 3500));
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

export async function loadCollections() {
  try {
    app.collections = await api.listCollections();
  } catch (e) {
    errorToast(e);
  }
}

export async function loadPending() {
  try {
    app.pending = await api.pendingMoves();
  } catch {
    app.pending = [];
  }
  if (!app.pending.length) app.showPending = false;
}

export async function loadHistory() {
  try {
    app.history = await api.historyList();
  } catch {
    app.history = [];
  }
}

/** The newest change that can still be undone. */
export function latestUndoable(): HistoryEntry | undefined {
  return app.history.find((h) => h.undoable && !h.undone);
}

/** Undo a change and everything after it, or just the latest change without an id. */
export async function undo(id?: number) {
  if (app.undoing) return;
  if (id === undefined && !latestUndoable()) {
    await loadHistory();
    if (!latestUndoable()) return toast("There is nothing to undo.", "info");
  }
  app.undoing = true;
  try {
    const r = await api.historyUndo(id);
    const what = r.steps > 1 ? `${r.steps} changes` : `“${r.label ?? "the last change"}”`;
    if (r.failed.length) {
      const first = r.failed.slice(0, 3).map(([f, e]) => `${f}: ${e}`).join("\n");
      toast(`Undid ${what}, but ${r.failed.length} item${r.failed.length === 1 ? "" : "s"} could not be put back.\n${first}`, "error", {
        label: "Show history",
        run: () => (app.showHistory = true),
      });
    } else {
      toast(`Undid ${what}.`, "success");
    }
  } catch (e) {
    errorToast(e);
  } finally {
    app.undoing = false;
    await afterFileChange();
  }
}

/** Show a toast with an Undo button for a change that just happened. */
export function offerUndo(history: number | null | undefined, text: string) {
  if (!history) return toast(text, "success");
  toast(text, "success", { label: "Undo", run: () => undo(history) }, 10000);
}

/** Refresh everything a file change can affect. */
async function afterFileChange() {
  await Promise.all([loadRoots(), loadPhotos(), loadPending(), loadHistory(), loadCollections()]);
  app.folderVersion++;
}

export async function cancelPending(ids: number[]) {
  try {
    const n = await api.cancelPendingMoves(ids);
    if (n) toast(`${n} move${n === 1 ? "" : "s"} cancelled. ${n === 1 ? "The photo is" : "The photos are"} back where ${n === 1 ? "it is" : "they are"} on disk.`, "success");
    await loadPending();
    await loadPhotos();
    app.folderVersion++;
  } catch (e) {
    errorToast(e);
  }
}

/** The worker finished (or couldn't finish) moves that were waiting for a drive. */
function pendingApplied(r: PendingReport) {
  if (r.done) toast(`${r.done} waiting move${r.done === 1 ? " is" : "s are"} done now that the drive is connected.`, "success");
  if (r.failed.length) {
    const first = r.failed.slice(0, 3).map(([f, e]) => `${f}: ${e}`).join("\n");
    toast(`${r.failed.length} waiting move${r.failed.length === 1 ? "" : "s"} couldn't be done yet.\n${first}`, "error", {
      label: "Show",
      run: () => (app.showPending = true),
    });
  }
  loadPending();
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
      collection: s.kind === "collection" ? s.id : null,
      text: app.text,
      min_rating: app.minRating,
      label: app.label,
      sort: s.kind === "ai" && app.sort === "relevance" ? undefined : app.sort,
      desc: app.desc,
    });
    if (seq !== loadSeq) return;
    const before = app.photos;
    app.allPhotos = photos;
    app.photos = collapseStacks(photos);
    saveTabs();
    const ids = new Set(app.photos.map((p) => p.id));
    // The focused photo left the list (moved or deleted): keep the place by moving to
    // the next photo that is still here, or the one before it.
    if (app.focusId !== null && !ids.has(app.focusId)) {
      const at = before.findIndex((p) => p.id === app.focusId);
      if (at >= 0) {
        const next = before.slice(at + 1).find((p) => ids.has(p.id)) ?? before.slice(0, at).reverse().find((p) => ids.has(p.id));
        if (next) {
          selection.clear();
          selection.add(next.id);
          app.focusId = next.id;
          app.anchorId = next.id;
        }
      }
    }
    for (const id of [...selection]) if (!ids.has(id)) selection.delete(id);
    if (app.viewerIndex >= app.photos.length) app.viewerIndex = app.photos.length - 1;
  } catch (e) {
    errorToast(e);
  } finally {
    if (seq === loadSeq) app.loading = false;
  }
}

// ------------------------------------------------------------------ stacks (groups of shots)

/**
 * Closed stacks show one photo: the cover, or the first match if the cover is filtered out.
 * Open stacks show every photo, kept together with the cover first.
 */
export function collapseStacks(list: PhotoLite[]): PhotoLite[] {
  const rep = new Map<number, number>();
  const groups = new Map<number, PhotoLite[]>();
  for (const p of list) {
    if (p.stack_id == null) continue;
    if (openStacks.has(p.stack_id)) {
      const g = groups.get(p.stack_id);
      if (g) g.push(p); else groups.set(p.stack_id, [p]);
    } else if (!rep.has(p.stack_id) || p.stack_top) rep.set(p.stack_id, p.id);
  }
  const out: PhotoLite[] = [];
  for (const p of list) {
    if (p.stack_id == null) out.push(p);
    else if (openStacks.has(p.stack_id)) {
      const g = groups.get(p.stack_id);
      if (!g) continue;
      groups.delete(p.stack_id);
      out.push(...g.filter((x) => x.stack_top), ...g.filter((x) => !x.stack_top));
    } else if (rep.get(p.stack_id) === p.id) out.push(p);
  }
  return out;
}

/** Is this photo standing in for a closed stack? */
export function isClosedStack(p: PhotoLite) {
  return p.stack_id != null && !openStacks.has(p.stack_id);
}

/** The ids plus every hidden photo in closed stacks among them (so groups move together). */
export function withStacks(ids: number[]): number[] {
  const want = new Set(ids);
  const stacks = new Set(app.photos.filter((p) => want.has(p.id) && isClosedStack(p)).map((p) => p.stack_id));
  if (!stacks.size) return ids;
  const out = new Set(ids);
  for (const p of app.allPhotos) if (p.stack_id != null && stacks.has(p.stack_id)) out.add(p.id);
  return [...out];
}

export function toggleStack(stackId: number, open?: boolean) {
  if (open ?? !openStacks.has(stackId)) openStacks.add(stackId);
  else {
    openStacks.delete(stackId);
    // Keep the selection on the cover rather than on photos that are now hidden.
    const members = app.allPhotos.filter((p) => p.stack_id === stackId);
    if (members.some((p) => selection.has(p.id))) {
      for (const p of members) selection.delete(p.id);
      const cover = members.find((p) => p.stack_top) ?? members[0];
      if (cover) { selection.add(cover.id); app.focusId = cover.id; }
    }
  }
  app.photos = collapseStacks(app.allPhotos);
}

export async function groupPhotos(ids: number[]) {
  const all = withStacks(ids);
  if (all.length < 2) { toast("Select two or more photos to group them.", "info"); return; }
  const top = app.focusId != null && all.includes(app.focusId) ? app.focusId : all[0];
  try {
    const sid = await api.stackPhotos(all, top);
    openStacks.delete(sid);
    selection.clear();
    selection.add(top);
    app.focusId = top;
    await loadPhotos();
    toast(`Grouped ${all.length} photos. Click the number on the photo, or press S, to open the group.`, "success");
  } catch (e) {
    errorToast(e);
  }
}

/** Dissolve the groups these photos are in (`whole`), or just take these photos out. */
export async function ungroupPhotos(ids: number[], whole = true) {
  try {
    await api.unstackPhotos(withStacks(ids), whole);
    await loadPhotos();
  } catch (e) {
    errorToast(e);
  }
}

export async function makeCover(id: number) {
  try {
    await api.setStackTop(id);
    await loadPhotos();
  } catch (e) {
    errorToast(e);
  }
}

// ------------------------------------------------------------------ rotate & flip

const TURN_WORDS: Record<Turn, string> = { left: "rotated", right: "rotated", flip_h: "flipped", flip_v: "flipped" };

export async function rotatePhotos(ids: number[], op: Turn) {
  if (!ids.length) return;
  try {
    const r = await api.rotatePhotos(ids, op);
    if (r.failed.length) reportResult(r, TURN_WORDS[op]);
    await loadPhotos();
  } catch (e) {
    errorToast(e);
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
  saveTabs();
}

let refreshTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleRefresh() {
  if (refreshTimer) return;
  refreshTimer = setTimeout(async () => {
    refreshTimer = null;
    // Independent reloads, so they run side by side.
    await Promise.all([loadRoots(), loadPhotos(), loadPending(), loadCollections()]);
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
    if (st.sidebar_width) app.sidebarWidth = clampSidebar(Number(st.sidebar_width) || SIDEBAR_DEFAULT);
    if (st.panel) app.panel = st.panel === "none" ? null : (st.panel as Panel);
    restoreTabs(st);
    const status = await api.appStatus();
    app.exiftool = status.exiftool;
    await loadRoots();
    await loadPhotos();
    await loadCollections();
  } catch (e) {
    errorToast(e);
  }
  await listen("library-changed", scheduleRefresh);
  await listen<PendingReport>("pending-moves", (e) => pendingApplied(e.payload));
  loadPending();
  loadHistory();
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
      if (changed && app.pending.length) api.retryPendingMoves().catch(() => {});
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
  const want = new Set(ids);
  for (const p of app.photos) {
    if (!want.has(p.id)) continue;
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
    const r = await api.movePhotos(withStacks(ids), d, copy);
    if (r.failed.length || !r.done) reportResult(r, copy ? "copied" : "moved");
    else offerUndo(r.history, `${r.done} photo${r.done === 1 ? "" : "s"} ${copy ? "copied" : "moved"}.`);
    if (r.queued) {
      toast(
        `${r.queued} photo${r.queued === 1 ? "" : "s"} moved in your library. The file${r.queued === 1 ? "" : "s"} will follow when the drive is connected.`,
        "info",
        { label: "Show waiting moves", run: () => (app.showPending = true) },
      );
      loadPending();
    }
    if (r.outside_library && r.done) {
      toast(`That folder isn't in your library${copy ? "" : ", so the moved photos left the catalogue"}.`, "info", {
        label: "Add folder to library",
        run: () => addFolderToLibrary(d),
      });
    }
    await afterFileChange();
  } catch (e) {
    errorToast(e);
  }
}

/** Move photos to the Trash after asking. Returns false if the user said no. */
export async function trashPhotos(ids: number[], message?: string): Promise<boolean> {
  if (!ids.length) return false;
  const n = ids.length;
  const ok = await confirm(
    `Move ${n} photo${n === 1 ? "" : "s"} to the Trash?`,
    message ?? "Their XMP sidecar files go too. You can restore them from the system Trash / Recycle Bin.",
    "Move to Trash",
    true,
  );
  if (!ok) return false;
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
        // Retry exactly the photos that failed, by id: two photos in different folders
        // can share a file name, and the wrong one must never be deleted permanently.
        const retry = r.failed_ids ?? [];
        if (retry.length !== r.failed.length) throw new Error("Lumio could not tell which photos failed; nothing more was deleted.");
        const r2 = await api.deletePhotos(retry, true);
        r = { done: r.done + r2.done, failed: r2.failed, failed_ids: r2.failed_ids, outside_library: false };
      }
    }
    reportResult(r, "deleted");
    selection.clear();
    await afterFileChange();
  } catch (e) {
    errorToast(e);
  }
  return true;
}

export async function renamePhoto(p: PhotoLite) {
  const name = await prompt("Rename photo", p.filename, "Rename", "The extension is kept if you leave it off. Its XMP sidecar is renamed too.");
  if (!name || name === p.filename) return;
  try {
    const r = await api.renamePhoto(p.id, name);
    await loadPhotos();
    loadHistory();
    if (r.done) offerUndo(r.history, `Renamed to “${app.photos.find((x) => x.id === p.id)?.filename ?? name}”.`);
  } catch (e) {
    errorToast(e);
  }
}

/** Rename what is selected: one photo by typing a name, several with the batch rename window. */
export function renameSelected(ids = selectedIds()) {
  if (!ids.length) return;
  const want = new Set(ids);
  const offline = app.photos.some((p) => want.has(p.id) && !rootOnline(p.root_id));
  if (offline) { toast("Connect the drive these photos are on to rename them.", "info"); return; }
  if (ids.length === 1) {
    const p = app.photos.find((x) => x.id === ids[0]);
    if (p) renamePhoto(p);
  } else app.renameIds = [...ids];
}

export async function newFolder(parent: string) {
  const name = await prompt("New folder", "", "Create", `Inside ${parent}`);
  if (!name) return;
  try {
    await api.createFolder(parent, name);
    app.folderVersion++;
    loadHistory();
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
    await afterFileChange();
    const h = latestUndoable();
    offerUndo(h?.kind === "folder_rename" ? h.id : null, `Folder renamed to “${name}”.`);
  } catch (e) {
    errorToast(e);
  }
}

/** `path` is `folder` itself or somewhere inside it ("/Trip 2" is not inside "/Trip"). */
export function isInside(path: string, folder: string): boolean {
  if (path === folder) return true;
  const f = folder.replace(/[\\/]+$/, "");
  return path.startsWith(f + "/") || path.startsWith(f + "\\");
}

export async function trashFolder(path: string, name: string) {
  const ok = await confirm(
    `Move the folder “${name}” to the Trash?`,
    "Everything inside it, including photos, goes to the system Trash / Recycle Bin.",
    "Move to Trash",
    true,
  );
  if (!ok) return;
  let permanent = false;
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
    permanent = true;
  }
  if (app.scope.kind === "folder" && isInside(app.scope.path, path)) setScope({ kind: "all" });
  toast(permanent ? `“${name}” deleted permanently.` : `“${name}” moved to the Trash.`, "success");
  app.folderVersion++;
  loadHistory();
}

export async function dropOnFolder(dest: string, e: DragEvent) {
  const copy = e.altKey;
  const photoData = e.dataTransfer?.getData("application/x-lumio-photos");
  const folderData = e.dataTransfer?.getData("application/x-lumio-folder");
  if (photoData) {
    const ids = JSON.parse(photoData) as number[];
    await moveOrCopy(ids, copy, dest);
  } else if (folderData) {
    if (isInside(dest, folderData)) return;
    try {
      const r = await api.moveFolder(folderData, dest, copy);
      if (r.done) offerUndo(r.history, `Folder ${copy ? "copied" : "moved"}.`);
      if (r.outside_library) toast("The destination isn't in your library.", "info");
      await afterFileChange();
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

// ------------------------------------------------------------------ slideshow

/** Play these photos as a slideshow, starting with the one at `start`. */
export function startSlideshow(photos: PhotoLite[], title: string, start = 0) {
  if (!photos.length) { toast("There are no photos to show.", "info"); return; }
  // Started from the viewer: go back to it, on the last photo shown, when the show ends.
  const backToViewer = app.viewerIndex >= 0;
  app.viewerIndex = -1;
  app.slideshow = { photos: [...photos], title, start: Math.max(0, Math.min(start, photos.length - 1)), backToViewer };
}

/**
 * The Slideshow button: with two or more photos selected it plays those (a selected
 * group plays every photo in it); otherwise it plays everything in view, starting
 * from the selected photo.
 */
export function slideshowSelectionOrView() {
  const ids = selectedIds();
  if (ids.length > 1) {
    const all = new Set(withStacks(ids));
    startSlideshow(app.allPhotos.filter((p) => all.has(p.id)), `${all.size} selected photos`);
    return;
  }
  const at = ids.length === 1 ? app.photos.findIndex((p) => p.id === ids[0]) : 0;
  startSlideshow(app.photos, scopeTitle(app.scope), Math.max(0, at));
}

/** Every photo in a group, cover first. */
export function slideshowGroup(stackId: number) {
  const members = app.allPhotos.filter((p) => p.stack_id === stackId);
  startSlideshow([...members.filter((p) => p.stack_top), ...members.filter((p) => !p.stack_top)], "Group");
}

/** A folder and its subfolders, in the current sort order. Groups show their cover. */
export async function slideshowFolder(path: string, name: string) {
  try {
    const photos = await api.queryPhotos({
      scope: "folder", folder: path, recursive: true, ids: null, text: "", min_rating: 0, label: "",
      sort: app.sort === "relevance" ? "date" : app.sort, desc: app.desc,
    });
    startSlideshow(collapseStacks(photos), name);
  } catch (e) {
    errorToast(e);
  }
}

export async function slideshowCollection(c: Collection) {
  try {
    const photos = await api.queryPhotos({
      scope: "collection", collection: c.id, text: "", min_rating: 0, label: "",
      sort: app.sort === "relevance" ? "date" : app.sort, desc: app.desc,
    });
    if (!photos.length) { toast("No photos in this collection yet."); return; }
    startSlideshow(collapseStacks(photos), c.name);
  } catch (e) {
    errorToast(e);
  }
}

export function scopeTitle(s: Scope): string {
  switch (s.kind) {
    case "all": return "All photos";
    case "recent": return "Recently added";
    case "rated": return "Rated";
    case "geotagged": return "On the map";
    case "nogps": return "Without location";
    case "folder": return s.name;
    case "collection": return s.name;
    case "ai": return s.title;
  }
}

export function setView(v: View) {
  app.view = v;
  saveTabs();
}

// ------------------------------------------------------------------ tabs

let tabSeq = 1;

function snapshot(id: number): Tab {
  return {
    id,
    scope: $state.snapshot(app.scope) as Scope,
    recursive: app.recursive,
    text: app.text,
    minRating: app.minRating,
    label: app.label,
    sort: app.sort,
    desc: app.desc,
    view: app.view,
    focusId: app.focusId,
  };
}

function applyTab(t: Tab) {
  app.scope = t.scope;
  app.recursive = t.recursive;
  app.text = t.text;
  app.minRating = t.minRating;
  app.label = t.label;
  app.sort = t.sort;
  app.desc = t.desc;
  app.view = t.view;
  selection.clear();
  app.focusId = t.focusId;
  app.anchorId = t.focusId;
  if (t.focusId !== null) selection.add(t.focusId);
}

function restoreTabs(st: Record<string, string>) {
  try {
    const tabs = JSON.parse(st.tabs || "[]") as Tab[];
    if (Array.isArray(tabs) && tabs.length) {
      tabs.forEach((t) => (t.id = tabSeq++));
      app.tabs = tabs;
      app.activeTab = Math.min(Math.max(0, Number(st.active_tab) || 0), tabs.length - 1);
      applyTab(tabs[app.activeTab]);
      return;
    }
  } catch {
    /* fall through to a fresh tab */
  }
  app.tabs = [snapshot(tabSeq++)];
  app.activeTab = 0;
}

let saveTabsTimer: ReturnType<typeof setTimeout> | undefined;
/** Remember the current tab's state and persist all tabs (debounced). */
export function saveTabs() {
  const cur = app.tabs[app.activeTab];
  if (cur) app.tabs[app.activeTab] = snapshot(cur.id);
  clearTimeout(saveTabsTimer);
  saveTabsTimer = setTimeout(() => {
    // AI result tabs can hold thousands of ids; store them, they're only numbers.
    saveSetting("tabs", JSON.stringify($state.snapshot(app.tabs)));
    saveSetting("active_tab", String(app.activeTab));
  }, 400);
}

export function switchTab(i: number) {
  if (i === app.activeTab || i < 0 || i >= app.tabs.length) return;
  saveTabs();
  app.activeTab = i;
  applyTab(app.tabs[i]);
  app.viewerIndex = -1;
  loadPhotos();
  saveTabs();
}

/** Open a new tab. Without a scope it starts as a copy of the current tab. */
export function newTab(scope?: Scope) {
  saveTabs();
  const t = snapshot(tabSeq++);
  if (scope) {
    t.scope = scope;
    t.text = "";
    t.minRating = 0;
    t.label = "";
    t.focusId = null;
    if (t.view === "map") t.view = "grid";
  }
  app.tabs.splice(app.activeTab + 1, 0, t);
  app.activeTab += 1;
  applyTab(t);
  loadPhotos();
  saveTabs();
}

export function closeTab(i: number) {
  if (app.tabs.length <= 1) return;
  saveTabs();
  const wasActive = i === app.activeTab;
  app.tabs.splice(i, 1);
  if (i < app.activeTab) app.activeTab--;
  else if (wasActive) {
    app.activeTab = Math.min(i, app.tabs.length - 1);
    applyTab(app.tabs[app.activeTab]);
    app.viewerIndex = -1;
    loadPhotos();
  }
  saveTabs();
}

export function moveTab(from: number, to: number) {
  if (from === to) return;
  saveTabs();
  const activeId = app.tabs[app.activeTab].id;
  const [t] = app.tabs.splice(from, 1);
  app.tabs.splice(to, 0, t);
  app.activeTab = app.tabs.findIndex((x) => x.id === activeId);
  saveTabs();
}

/** Photos dropped onto a tab go into that tab's folder. */
export async function dropOnTab(i: number, e: DragEvent) {
  const t = app.tabs[i];
  if (!t || t.scope.kind !== "folder") {
    toast("Open a folder in that tab to drop photos into it.", "info");
    return;
  }
  await dropOnFolder(t.scope.path, e);
}

/** Photos dropped onto the photo area go into the folder this tab is showing. */
export async function dropOnView(e: DragEvent) {
  if (app.scope.kind !== "folder") return;
  const data = e.dataTransfer?.getData("application/x-lumio-photos");
  if (!data) return;
  const ids = JSON.parse(data) as number[];
  const dest = app.scope.path;
  // Dropping photos back where they already are does nothing.
  if (ids.every((id) => app.photos.some((p) => p.id === id && p.folder === dest))) return;
  await moveOrCopy(ids, e.altKey, dest);
}

// ------------------------------------------------------------------ smart collections

/** Open the editor for a new collection, optionally starting from some rules. */
export function newCollection(name = "", rules: RuleSet = { any: false, rules: [] }) {
  app.collectionEdit = { id: null, name, rules: $state.snapshot(rules) as RuleSet };
}

export function editCollection(c: Collection) {
  app.collectionEdit = { id: c.id, name: c.name, rules: $state.snapshot(c.rules) as RuleSet };
}

/** Save the collection in the editor and show it. */
export async function saveCollection(id: number | null, name: string, rules: RuleSet): Promise<boolean> {
  try {
    const saved = await api.saveCollection(id, name, rules);
    app.collectionEdit = null;
    await loadCollections();
    const now = { kind: "collection", id: saved, name: name.trim() } as const;
    // Tabs showing this collection pick up a new name.
    app.tabs.forEach((t) => { if (t.scope.kind === "collection" && t.scope.id === saved) t.scope = { ...now }; });
    if (app.scope.kind === "collection" && app.scope.id === saved) { app.scope = { ...now }; loadPhotos(); saveTabs(); }
    else setScope(now);
    return true;
  } catch (e) {
    errorToast(e);
    return false;
  }
}

export async function deleteCollection(c: Collection) {
  const ok = await confirm(`Delete the collection “${c.name}”?`, "Only the saved rules are deleted. Your photos stay exactly as they are.", "Delete");
  if (!ok) return;
  try {
    await api.deleteCollection(c.id);
    await loadCollections();
    if (app.scope.kind === "collection" && app.scope.id === c.id) setScope({ kind: "all" });
  } catch (e) {
    errorToast(e);
  }
}

export function duplicateCollection(c: Collection) {
  newCollection(`${c.name} copy`, c.rules);
}

/** Open the duplicate finder for the whole library, or for one folder and its subfolders. */
export function findDuplicates(folder: string | null = null, name = "") {
  app.dupes = { folder, name };
}
