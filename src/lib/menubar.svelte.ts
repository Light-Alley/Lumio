// The native menu bar (built in src-tauri/src/appmenu.rs). Each item arrives here as a
// `menu` event with its id and runs the same code as the right-click menu, the toolbar
// or the keyboard shortcut. In the other direction, `syncMenu` keeps the menu honest:
// items that need a selection are greyed out, and views, panels and sorting show a tick.
//
// Shortcuts can arrive twice: macOS gives the web view the first go and only uses the menu
// when the web view didn't handle the key, but Windows and Linux may do it the other way
// round. So a menu event that matches a key the web view has just handled is ignored, and
// a key typed in a text box never triggers a photo action from the menu.
import { tick } from "svelte";
import { listen } from "@tauri-apps/api/event";
import { api, errText, isMac, LABELS, shareApi, REPO_URL, type MenuItemState, type PhotoLite } from "./api";
import {
  app, selection, selectedIds, rate, setLabel, moveOrCopy, trashPhotos, renameSelected, openViewer,
  errorToast, toast, newFolder, newTab, closeTab, switchTab, setView, togglePanel, setPanel, saveSetting,
  loadPhotos, rotatePhotos, groupPhotos, ungroupPhotos, toggleStack, slideshowSelectionOrView,
  addFolderToLibrary, newCollection, findDuplicates, undo, latestUndoable, rootOnline, type Bg, type View,
} from "./store.svelte";
import { chooseApp, copyPhotos, sharing } from "./actions";
import { openSmugMug, openWebExport } from "./publish.svelte";
import { setFullscreen, toggleFullscreen } from "./fullscreen";
import { checkForUpdates, updates } from "./updates.svelte";

export const shortcutsHelp = $state({ open: false });

const MOD = isMac ? "meta" : "ctrl";
/** The keys the menu uses as shortcuts, written the way `combo()` writes key presses. */
const ACCEL: Record<string, string> = {
  settings: `${MOD}+,`,
  new_tab: `${MOD}+t`,
  close_tab: `${MOD}+w`,
  add_folder: `${MOD}+o`,
  new_folder: `${MOD}+shift+n`,
  undo: `${MOD}+z`,
  select_all: `${MOD}+a`,
  find: `${MOD}+f`,
  thumbs_bigger: `${MOD}+=`,
  thumbs_smaller: `${MOD}+-`,
  fullscreen: isMac ? "ctrl+meta+f" : "f11",
  rotate_left: `${MOD}+[`,
  rotate_right: `${MOD}+]`,
  group: `${MOD}+g`,
  ungroup: `${MOD}+shift+g`,
  rename: "f2",
  trash: "meta+backspace",
  next_tab: "ctrl+tab",
  prev_tab: "ctrl+shift+tab",
  help_shortcuts: `${MOD}+/`,
};

function combo(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("ctrl");
  if (e.metaKey) parts.push("meta");
  if (e.altKey) parts.push("alt");
  if (e.shiftKey) parts.push("shift");
  parts.push(e.key === "+" ? "=" : e.key.toLowerCase());
  return parts.join("+");
}

function isTextField(el: Element | null): el is HTMLInputElement | HTMLTextAreaElement | HTMLElement {
  if (!el) return false;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) return !["checkbox", "radio", "range", "button", "submit", "color", "file"].includes(el.type);
  return (el as HTMLElement).isContentEditable;
}

/** The last key press: its combo, when, whether it was typed in a text box, and whether the page used it. */
let lastKey = { combo: "", at: 0, inText: false, handled: false };

/** A dialog or window is open on top of the photos. */
function modalOpen() {
  return !!(app.dialog || app.renameIds || app.collectionEdit || app.dupes || app.showSettings || app.showHistory || app.showPending || app.slideshow || shortcutsHelp.open || updates.open);
}

/** The photos a menu command acts on: the one in the viewer, else the selection. */
function targets(): number[] {
  if (app.viewerIndex >= 0) {
    const p = app.photos[app.viewerIndex];
    return p ? [p.id] : [];
  }
  return selectedIds();
}
function targetPhotos(): PhotoLite[] {
  const ids = new Set(targets());
  return app.allPhotos.filter((p) => ids.has(p.id));
}
function need(ids: number[]): boolean {
  if (ids.length) return true;
  toast("Select one or more photos first.");
  return false;
}

const SORTS = ["date", "name", "rating", "modified", "size"];
const BGS: Bg[] = ["black", "grey", "lightgrey", "white"];
const VIEWS: View[] = ["grid", "list", "filmstrip", "map"];

/** Close whatever covers the photos, so a menu command can show its result. */
function closeViewer() {
  if (app.viewerIndex >= 0) app.viewerIndex = -1;
}

async function focusInInspector(selector: string) {
  setPanel("info");
  await tick();
  const el = document.querySelector<HTMLInputElement>(`.inspector ${selector}`);
  if (el) { el.focus(); el.select?.(); }
}

function setThumbSize(d: number) {
  app.thumbSize = Math.max(100, Math.min(360, app.thumbSize + d));
  saveSetting("thumb_size", String(app.thumbSize));
}

function openPage(path: string) {
  api.openProjectPage(`${REPO_URL}/${path}`).catch(errorToast);
}

/** Run one menu command. */
export function runMenu(id: string) {
  const ids = targets();
  const shareAt = () => ({ x: Math.round(window.innerWidth / 2), y: 60 });

  if (id.startsWith("view_")) { closeViewer(); setView(id.slice(5) as View); return; }
  if (id.startsWith("sort_") && id !== "sort_desc") { app.sort = id.slice(5); loadPhotos(); return; }
  if (id.startsWith("bg_")) { app.viewerBg = id.slice(3) as Bg; saveSetting("viewer_bg", app.viewerBg); return; }
  if (id.startsWith("theme_")) { app.theme = id.slice(6) as "dark" | "light"; saveSetting("theme", app.theme); return; }
  if (id.startsWith("rate_")) { if (need(ids)) rate(ids, Number(id.slice(5))); return; }
  if (id.startsWith("label_")) {
    if (!need(ids)) return;
    const name = id.slice(6);
    setLabel(ids, name === "none" ? "" : LABELS.find((l) => l.name.toLowerCase() === name)?.name ?? "");
    return;
  }

  switch (id) {
    // Frameloft / File
    case "settings": app.showSettings = true; break;
    case "check_updates": checkForUpdates(true); break;
    case "new_tab": newTab(); break;
    case "close_tab":
      if (app.viewerIndex >= 0) closeViewer();
      else if (app.tabs.length > 1) closeTab(app.activeTab);
      break;
    case "next_tab": if (app.tabs.length > 1) switchTab((app.activeTab + 1) % app.tabs.length); break;
    case "prev_tab": if (app.tabs.length > 1) switchTab((app.activeTab - 1 + app.tabs.length) % app.tabs.length); break;
    case "add_folder": addFolderToLibrary(); break;
    case "new_folder":
      if (app.scope.kind === "folder") newFolder(app.scope.path);
      else toast("Open a folder first, then choose New Folder to make one inside it.");
      break;
    case "rescan": api.rescan().then(() => toast("Rescanning your library…")).catch(errorToast); break;
    case "new_collection": newCollection(); break;
    case "find_duplicates": closeViewer(); findDuplicates(); break;
    case "open_default": if (need(ids)) api.openDefault(ids).catch(errorToast); break;
    case "open_with": if (need(ids)) chooseApp(ids); break;
    case "reveal": if (need(ids)) api.reveal(ids[0]).catch(errorToast); break;
    case "export_web": if (need(ids)) openWebExport(ids); break;
    case "smugmug": if (need(ids)) openSmugMug(ids); break;

    // Edit
    case "undo":
      if (isTextField(document.activeElement)) document.execCommand("undo");
      else undo();
      break;
    case "select_all": {
      const el = document.activeElement;
      if (isTextField(el) && "select" in el) (el as HTMLInputElement).select();
      else if (app.viewerIndex < 0) for (const p of app.photos) selection.add(p.id);
      break;
    }
    case "select_none": selection.clear(); break;
    case "copy_photos": if (need(ids)) copyPhotos(ids); break;
    case "find": {
      closeViewer();
      const el = document.querySelector<HTMLInputElement>(".toolbar input[type=search]");
      el?.focus();
      el?.select();
      break;
    }
    case "history": app.showHistory = true; break;
    case "pending": app.showPending = true; break;

    // View
    case "panel_info": togglePanel("info"); break;
    case "panel_ai": togglePanel("ai"); break;
    case "histogram": {
      const on = app.settings.inspector_preview === "histogram";
      saveSetting("inspector_preview", on ? "photo" : "histogram");
      if (!on) setPanel("info");
      break;
    }
    case "recursive":
      if (app.scope.kind !== "folder") { toast("Include Subfolders applies when a folder is open."); break; }
      app.recursive = !app.recursive;
      saveSetting("recursive", app.recursive ? "1" : "0");
      loadPhotos();
      break;
    case "sort_desc": app.desc = !app.desc; loadPhotos(); break;
    case "thumbs_bigger": setThumbSize(20); break;
    case "thumbs_smaller": setThumbSize(-20); break;
    case "slideshow": if (app.photos.length) { closeViewer(); slideshowSelectionOrView(); } break;
    case "fullscreen": toggleFullscreen(); break;

    // Photo
    case "open_viewer": if (need(ids) && app.viewerIndex < 0) openViewer(app.focusId ?? ids[0]); break;
    case "view_fullscreen":
      if (!need(ids)) break;
      if (app.viewerIndex < 0) openViewer(app.focusId ?? ids[0]);
      setFullscreen(true);
      break;
    case "edit_keywords": if (need(ids)) { closeViewer(); focusInInspector("input.kw"); } break;
    case "edit_details": if (need(ids)) { closeViewer(); focusInInspector("input[type=text]"); } break;
    case "rotate_left": if (need(ids)) rotatePhotos(ids, "left"); break;
    case "rotate_right": if (need(ids)) rotatePhotos(ids, "right"); break;
    case "flip_h": if (need(ids)) rotatePhotos(ids, "flip_h"); break;
    case "flip_v": if (need(ids)) rotatePhotos(ids, "flip_v"); break;
    case "group":
      if (ids.length < 2) toast("Select two or more photos to group them.");
      else groupPhotos(ids);
      break;
    case "ungroup": if (need(ids)) ungroupPhotos(ids, true); break;
    case "toggle_group": {
      const p = app.photos.find((x) => x.id === app.focusId);
      if (p?.stack_id != null) toggleStack(p.stack_id);
      else toast("The selected photo isn't in a group.");
      break;
    }
    case "rename": if (need(ids)) { closeViewer(); renameSelected(ids); } break;
    case "copy_to": if (need(ids)) moveOrCopy(ids, true); break;
    case "move_to": if (need(ids)) moveOrCopy(ids, false); break;
    case "share_sheet": if (need(ids)) { const { x, y } = shareAt(); sharing(() => shareApi.sheet(ids, x, y)); } break;
    case "share_email": if (need(ids)) sharing(() => shareApi.email(ids)); break;
    case "share_whatsapp": if (need(ids)) sharing(() => shareApi.whatsapp(ids, false)); break;
    case "share_whatsapp_web": if (need(ids)) sharing(() => shareApi.whatsapp(ids, true)); break;
    case "share_show": if (need(ids)) sharing(() => shareApi.show(ids)); break;
    case "reread": if (need(ids)) api.refreshPhotos(ids).then(() => toast("Refreshing…")).catch(errorToast); break;
    case "trash": if (need(ids)) trashPhotos(ids); break;

    // Help
    case "help_guide": openPage("blob/main/docs/user-guide.md"); break;
    case "help_shortcuts": shortcutsHelp.open = true; break;
    case "help_releases": openPage("releases"); break;
    case "help_issue": openPage("issues"); break;
    case "help_log": api.showLog().catch(errorToast); break;
    default: console.warn("unknown menu item", id);
  }
}

/** Commands that also make sense while typing (they act on the text box or don't touch photos). */
const FINE_IN_TEXT = new Set(["undo", "select_all", "settings", "new_tab", "close_tab", "find", "fullscreen", "help_shortcuts", "add_folder", "thumbs_bigger", "thumbs_smaller", "next_tab", "prev_tab"]);
/** Menu-only shortcuts, run here too because Windows doesn't pass keys to the menu while the page has focus. */
const PAGE_KEYS = new Set(["settings", "add_folder", "new_folder", "find", "thumbs_bigger", "thumbs_smaller", "fullscreen", "help_shortcuts", "next_tab", "prev_tab"]);
let lastRun = { id: "", at: 0 };

function fromMenu(id: string) {
  const now = Date.now();
  const accel = ACCEL[id];
  const fromKey = !!accel && lastKey.combo === accel && now - lastKey.at < 500;
  if (fromKey && lastKey.handled) return; // the page already did it
  if (lastRun.id === id && now - lastRun.at < 500) return;
  if (fromKey && lastKey.inText && !FINE_IN_TEXT.has(id)) return; // e.g. Cmd+Backspace while typing
  if (modalOpen() && !["settings", "fullscreen", "help_shortcuts", "help_guide", "help_releases", "help_issue", "help_log", "check_updates", "undo", "select_all"].includes(id)) return;
  lastRun = { id, at: now };
  try {
    runMenu(id);
  } catch (e) {
    toast(errText(e), "error");
  }
}

function onKeyDown(e: KeyboardEvent) {
  const c = combo(e);
  lastKey = { combo: c, at: Date.now(), inText: isTextField(e.target as Element), handled: false };
  // See after the other handlers have run whether one of them used the key.
  setTimeout(() => { if (lastKey.combo === c) lastKey.handled = e.defaultPrevented; }, 0);
  const id = Object.keys(ACCEL).find((k) => ACCEL[k] === c);
  if (!id || !PAGE_KEYS.has(id) || e.defaultPrevented) return;
  if (id === "new_folder" && (lastKey.inText || modalOpen())) return;
  if (["find", "thumbs_bigger", "thumbs_smaller", "add_folder", "next_tab", "prev_tab"].includes(id) && modalOpen()) return;
  e.preventDefault();
  lastRun = { id, at: Date.now() };
  runMenu(id);
}

// ------------------------------------------------------------------ keeping the menu in step

let sent = new Map<string, string>();
let syncTimer: ReturnType<typeof setTimeout> | undefined;

function wanted(): MenuItemState[] {
  const ids = targets();
  const photos = targetPhotos();
  const n = ids.length;
  const any = n > 0;
  const files = photos.some((p) => rootOnline(p.root_id) && !p.pending);
  const one = n === 1 ? photos[0] : undefined;
  const modal = modalOpen();
  const free = !modal;
  const latest = latestUndoable();
  const focus = app.photos.find((x) => x.id === app.focusId);
  const many = n > 1 ? ` ${n} Photos` : "";
  const out: MenuItemState[] = [
    { id: "close_tab", enabled: app.tabs.length > 1 || app.viewerIndex >= 0 },
    { id: "next_tab", enabled: app.tabs.length > 1 },
    { id: "prev_tab", enabled: app.tabs.length > 1 },
    { id: "new_folder", enabled: free && app.scope.kind === "folder" && (app.scope.rootId == null || rootOnline(app.scope.rootId)) },
    { id: "rescan", enabled: app.roots.length > 0 },
    { id: "find_duplicates", enabled: free && app.roots.length > 0 },
    { id: "undo", enabled: true, text: latest && !app.undoing ? `Undo ${latest.label}` : "Undo" },
    { id: "select_all", enabled: true },
    { id: "select_none", enabled: free && selection.size > 0 },
    { id: "pending", enabled: app.pending.length > 0 },
    { id: "slideshow", enabled: free && app.photos.length > 0 },
    { id: "fullscreen", checked: app.fullscreen },
    { id: "panel_info", checked: app.panel === "info" },
    { id: "panel_ai", checked: app.panel === "ai" },
    { id: "histogram", checked: app.settings.inspector_preview === "histogram" },
    { id: "recursive", enabled: app.scope.kind === "folder", checked: app.scope.kind === "folder" && app.recursive },
    { id: "sort_desc", checked: app.desc },
    { id: "thumbs_bigger", enabled: app.thumbSize < 360 },
    { id: "thumbs_smaller", enabled: app.thumbSize > 100 },
    { id: "toggle_group", enabled: free && focus?.stack_id != null },
    { id: "group", enabled: free && n > 1 },
    { id: "ungroup", enabled: free && photos.some((p) => p.stack_id != null) },
    { id: "rename", enabled: free && any && files, text: n > 1 ? `Rename${many}…` : "Rename…" },
    { id: "trash", enabled: free && any && files, text: n > 1 ? `Move${many} to Trash` : "Move to Trash" },
    { id: "move_to", enabled: free && any, text: n > 1 ? `Move${many} To…` : "Move To…" },
    { id: "copy_to", enabled: free && any && files, text: n > 1 ? `Copy${many} To…` : "Copy To…" },
  ];
  for (const v of VIEWS) out.push({ id: `view_${v}`, checked: app.view === v && app.viewerIndex < 0 });
  for (const s of SORTS) out.push({ id: `sort_${s}`, checked: app.sort === s });
  for (const b of BGS) out.push({ id: `bg_${b}`, checked: app.viewerBg === b });
  out.push({ id: "theme_dark", checked: app.theme === "dark" }, { id: "theme_light", checked: app.theme === "light" });
  for (const r of [0, 1, 2, 3, 4, 5]) out.push({ id: `rate_${r}`, enabled: free && any, checked: !!one && one.rating === r });
  for (const l of ["none", ...LABELS.map((x) => x.name.toLowerCase())]) {
    out.push({ id: `label_${l}`, enabled: free && any, checked: !!one && (l === "none" ? !one.label : one.label?.toLowerCase() === l) });
  }
  for (const id of ["open_viewer", "view_fullscreen", "edit_keywords", "edit_details", "copy_photos"]) out.push({ id, enabled: free && any });
  for (const id of ["open_default", "open_with", "reveal", "export_web", "smugmug", "rotate_left", "rotate_right", "flip_h", "flip_v",
    "share_sheet", "share_email", "share_whatsapp", "share_whatsapp_web", "share_show", "reread"]) out.push({ id, enabled: free && any && files });
  return out;
}

function sync() {
  const changes = wanted().filter((s) => {
    const k = JSON.stringify(s);
    if (sent.get(s.id) === k) return false;
    sent.set(s.id, k);
    return true;
  });
  if (changes.length) api.menuSync(changes).catch(() => {});
}

let started = false;
/** Listen for menu commands and keep the menu bar's ticks and greyed-out items up to date. */
export function startMenubar() {
  if (started) return;
  started = true;
  window.addEventListener("keydown", onKeyDown, true);
  listen<string>("menu", (e) => {
    // A clicked check item has already ticked itself; send its real state again.
    sent.delete(e.payload);
    fromMenu(e.payload);
    clearTimeout(syncTimer);
    syncTimer = setTimeout(sync, 30);
  }).catch(() => {});
  $effect.root(() => {
    $effect(() => {
      wanted(); // read everything the menu depends on
      clearTimeout(syncTimer);
      syncTimer = setTimeout(sync, 60);
    });
  });
}
