// Keyboard and drag behaviour shared by the grid, list and filmstrip views.
import { app, selection, clickSelect, openViewer, rate, setLabel, trashPhotos, selectedIds } from "./store.svelte";
import { LABELS, type PhotoLite } from "./api";

/**
 * Handle the browsing shortcuts. `step` maps arrow keys to how far they move the focus
 * (e.g. a whole row in the grid). `reveal` scrolls the focused photo into view.
 */
export function browseKeys(e: KeyboardEvent, step: Record<string, number>, reveal: (index: number) => void) {
  if (app.viewerIndex >= 0 || app.dialog || app.showSettings || (e.target as HTMLElement)?.closest("input, textarea, select")) return;
  const n = app.photos.length;
  if (!n) return;
  const meta = e.metaKey || e.ctrlKey;
  let idx = app.photos.findIndex((p) => p.id === app.focusId);
  const ids = selectedIds();
  if (meta && e.key.toLowerCase() === "a") {
    e.preventDefault();
    for (const p of app.photos) selection.add(p.id);
    return;
  }
  if (e.key === "Escape") { selection.clear(); return; }
  if ((e.key === "Delete" || (e.key === "Backspace" && meta)) && ids.length) { e.preventDefault(); trashPhotos(ids); return; }
  if ((e.key === " " || e.key === "Enter") && app.focusId !== null) { e.preventDefault(); openViewer(app.focusId); return; }
  if (!meta && /^[0-5]$/.test(e.key) && ids.length) { rate(ids, Number(e.key)); return; }
  if (!meta && /^[6-9]$/.test(e.key) && ids.length) {
    const l = LABELS[Number(e.key) - 6].name;
    const allHave = app.photos.filter((p) => selection.has(p.id)).every((p) => p.label === l);
    setLabel(ids, allHave ? "" : l);
    return;
  }
  const moves: Record<string, number> = { ...step, Home: -n, End: n };
  if (e.key in moves) {
    e.preventDefault();
    if (idx < 0) idx = 0;
    else idx = Math.max(0, Math.min(n - 1, idx + moves[e.key]));
    clickSelect(app.photos[idx].id, e);
    reveal(idx);
  }
}

/** Start dragging the selection (or this photo). Drop targets: folders, tabs, other views. */
export function photoDragStart(e: DragEvent, p: PhotoLite) {
  if (!selection.has(p.id)) clickSelect(p.id, e as unknown as MouseEvent);
  const ids = selectedIds();
  app.dragIds = ids;
  app.dragPhotos = app.photos.filter((x) => selection.has(x.id));
  e.dataTransfer?.setData("application/x-lumio-photos", JSON.stringify(ids));
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = "copyMove";
    const ghost = document.createElement("div");
    ghost.textContent = ids.length === 1 ? p.filename : `${ids.length} photos`;
    ghost.style.cssText = "position:fixed;top:-100px;padding:4px 10px;border-radius:12px;background:#4f8cff;color:white;font:12px system-ui";
    document.body.appendChild(ghost);
    e.dataTransfer.setDragImage(ghost, 10, 10);
    setTimeout(() => ghost.remove(), 0);
  }
}

export function photoDragEnd() {
  app.dragIds = [];
  app.dragPhotos = [];
}

/** Dragged photos that are no longer in the current list (the drag started in another tab). */
export function dragGhosts(): PhotoLite[] {
  if (!app.dragPhotos.length) return [];
  const here = new Set(app.photos.map((p) => p.id));
  return app.dragPhotos.filter((p) => !here.has(p.id));
}

/** True while photos from Lumio are being dragged. */
export function isPhotoDrag(e: DragEvent) {
  return !!e.dataTransfer?.types.includes("application/x-lumio-photos");
}

/** Can the photos being dragged be dropped onto the photo area of this tab? */
export function canDropHere(e: DragEvent) {
  if (!isPhotoDrag(e) || app.scope.kind !== "folder") return false;
  const dest = app.scope.path;
  return app.dragIds.some((id) => !app.photos.some((p) => p.id === id && p.folder === dest));
}

// Clear drag state however the drag ends.
if (typeof window !== "undefined") {
  window.addEventListener("drop", () => setTimeout(photoDragEnd, 0));
  window.addEventListener("dragend", photoDragEnd);
}
