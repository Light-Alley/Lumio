// Context-menu builders shared by the grid, viewer and map.
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, errText, fileManagerName, isMac, isLinux, LABELS, shareApi, type PhotoLite, type FolderNode, type Root, type ShareOutcome } from "./api";
import { menu, type MenuItem } from "./menu.svelte";
import { setFullscreen } from "./fullscreen";
import { openSmugMug, openWebExport } from "./publish.svelte";
import {
  app, selection, selectedIds, rate, setLabel, moveOrCopy, trashPhotos, renameSelected, openViewer,
  errorToast, toast, newFolder, renameFolder, trashFolder, removeRoot, rootOnline, newTab,
  rotatePhotos, groupPhotos, ungroupPhotos, makeCover, toggleStack, openStacks,
  slideshowSelectionOrView, slideshowGroup, slideshowFolder,
} from "./store.svelte";

const cmd = isMac ? "⌘" : "Ctrl+";

export function photoMenu(p: PhotoLite): MenuItem[] {
  if (!selection.has(p.id)) {
    selection.clear();
    selection.add(p.id);
    app.focusId = p.id;
  }
  const ids = selectedIds();
  const many = ids.length > 1;
  const online = rootOnline(p.root_id);
  // A photo whose move is waiting for a drive isn't at its new place yet.
  const offlineNote = online && !p.pending ? undefined : true;
  return [
    { label: "Open in viewer", shortcut: "Space", action: () => openViewer(p.id) },
    { label: "View full screen", action: () => { openViewer(p.id); setFullscreen(true); } },
    { label: many ? `Slideshow of ${ids.length} photos` : "Slideshow from here", shortcut: "P", action: () => slideshowSelectionOrView() },
    { label: `Open in default app`, disabled: offlineNote, action: () => api.openDefault(ids).catch(errorToast) },
    {
      label: "Open with",
      disabled: offlineNote,
      submenu: async () => {
        try {
          const apps = await api.listOpenWith(p.id);
          const items: MenuItem[] = apps.map((a) => ({
            label: a.is_default ? `${a.name} (default)` : a.name,
            action: () => api.openWith(ids, a.id).catch(errorToast),
          }));
          items.push({ separator: true }, { label: "Other application…", action: () => chooseApp(ids) });
          return items;
        } catch (e) {
          return [{ label: errText(e), disabled: true }];
        }
      },
    },
    { label: `Show in ${fileManagerName}`, disabled: offlineNote, action: () => api.reveal(p.id).catch(errorToast) },
    { label: "Share", disabled: offlineNote, submenu: shareMenu(ids) },
    { separator: true },
    {
      label: "Rating",
      submenu: [0, 1, 2, 3, 4, 5].map((n) => ({
        label: n === 0 ? "No rating" : "★".repeat(n),
        shortcut: String(n),
        checked: !many && p.rating === n,
        action: () => rate(ids, n),
      })),
    },
    {
      label: "Colour label",
      submenu: [
        { label: "None", shortcut: "", checked: !many && !p.label, action: () => setLabel(ids, "") },
        ...LABELS.map((l, i) => ({
          label: l.name,
          color: l.color,
          shortcut: i < 4 ? String(i + 6) : "",
          checked: !many && p.label === l.name,
          action: () => setLabel(ids, l.name),
        })),
      ],
    },
    { separator: true },
    { label: "Rotate left", shortcut: `${cmd}[`, disabled: offlineNote, action: () => rotatePhotos(ids, "left") },
    { label: "Rotate right", shortcut: `${cmd}]`, disabled: offlineNote, action: () => rotatePhotos(ids, "right") },
    { label: "Flip horizontal", disabled: offlineNote, action: () => rotatePhotos(ids, "flip_h") },
    { label: "Flip vertical", disabled: offlineNote, action: () => rotatePhotos(ids, "flip_v") },
    { separator: true },
    ...stackItems(p, ids),
    { label: many ? `Copy ${ids.length} photos to…` : "Copy to…", disabled: offlineNote, action: () => moveOrCopy(ids, true) },
    // Moving works with the drive away too: the files follow when it's back.
    { label: many ? `Move ${ids.length} photos to…` : "Move to…", action: () => moveOrCopy(ids, false) },
    { label: "Upload to SmugMug…", disabled: offlineNote, action: () => openSmugMug(ids) },
    { label: "Export web-ready copies…", disabled: offlineNote, action: () => openWebExport(ids) },
    { label: many ? `Rename ${ids.length} photos…` : "Rename…", shortcut: "F2", disabled: offlineNote, action: () => renameSelected(ids) },
    {
      label: "Re-read metadata & thumbnail",
      disabled: offlineNote,
      action: () => api.refreshPhotos(ids).then(() => toast("Refreshing…")).catch(errorToast),
    },
    { separator: true },
    { label: many ? `Move ${ids.length} photos to Trash` : "Move to Trash", danger: true, shortcut: isMac ? "⌘⌫" : "Del", disabled: offlineNote, action: () => trashPhotos(ids) },
  ];
}

function stackItems(p: PhotoLite, ids: number[]): MenuItem[] {
  const items: MenuItem[] = [];
  const inStack = p.stack_id != null;
  if (ids.length > 1) items.push({ label: `Group ${ids.length} photos`, shortcut: `${cmd}G`, action: () => groupPhotos(ids) });
  if (inStack) {
    const open = openStacks.has(p.stack_id!);
    items.push({ label: open ? "Close group" : `Open group (${p.stack_count} photos)`, shortcut: "S", action: () => toggleStack(p.stack_id!) });
    if (open && !p.stack_top && ids.length === 1) items.push({ label: "Make cover of group", action: () => makeCover(p.id) });
    items.push({ label: `Slideshow of this group`, action: () => slideshowGroup(p.stack_id!) });
    if (open) items.push({ label: ids.length > 1 ? "Remove these from group" : "Remove from group", action: () => ungroupPhotos(ids, false) });
    items.push({ label: "Ungroup", shortcut: isMac ? "⇧⌘G" : "Ctrl+Shift+G", action: () => ungroupPhotos(ids, true) });
  }
  if (!items.length) items.push({ label: "Group (select two or more)", disabled: true });
  items.push({ separator: true });
  return items;
}

async function chooseApp(ids: number[]) {
  const app = await openDialog({
    title: "Choose an application",
    multiple: false,
    directory: false,
    defaultPath: isMac ? "/Applications" : undefined,
    filters: isMac ? [{ name: "Applications", extensions: ["app"] }] : /Windows/.test(navigator.userAgent) ? [{ name: "Programs", extensions: ["exe"] }] : undefined,
  });
  if (typeof app === "string") api.openWith(ids, app).catch(errorToast);
}

export function folderMenu(n: FolderNode, root?: Root): MenuItem[] {
  const items: MenuItem[] = [
    { label: "Open in new tab", action: () => newTab({ kind: "folder", path: n.path, name: n.name, rootId: root?.id }) },
    { label: "Slideshow", action: () => slideshowFolder(n.path, n.name) },
    { separator: true },
    { label: "New folder…", disabled: !n.online, action: () => newFolder(n.path) },
    { label: "Rename…", disabled: !n.online || !!root, action: () => renameFolder(n.path, n.name) },
    { label: `Show in ${fileManagerName}`, disabled: !n.online, action: () => api.reveal(undefined, n.path).catch(errorToast) },
    { label: "Rescan folder", disabled: !n.online, action: () => api.rescan(undefined, n.path).then(() => toast("Rescanning…")).catch(errorToast) },
    { separator: true },
  ];
  if (root) {
    items.push({ label: "Remove from library…", action: () => removeRoot(root) });
  } else {
    items.push({ label: "Move folder to Trash", danger: true, disabled: !n.online, action: () => trashFolder(n.path, n.name) });
  }
  return items;
}

// ------------------------------------------------------------------ sharing

/** Run a share action, showing what to do next (these hints stay up a little longer). */
function sharing(run: () => Promise<ShareOutcome>, busy = "Preparing…") {
  const slow = setTimeout(() => toast(busy), 600);
  run()
    .then((r) => { if (r.message) toast(r.message, "success", undefined, 9000); })
    .catch(errorToast)
    .finally(() => clearTimeout(slow));
}

/** The Share submenu, for the context menu and the toolbar's Share button. */
export function shareMenu(ids: number[]): MenuItem[] {
  const n = ids.length;
  const items: MenuItem[] = [];
  if (!isLinux) {
    items.push(
      { label: isMac ? "Share… (AirDrop, Messages, Mail and more)" : "Share… (Windows share)", action: () => {
        // Anchor the macOS share sheet where the menu was opened.
        const x = menu.x, y = menu.y;
        sharing(() => shareApi.sheet(ids, x, y));
      } },
      { separator: true },
    );
  }
  items.push(
    { label: "Email", action: () => sharing(() => shareApi.email(ids)) },
    { label: "WhatsApp", action: () => sharing(() => shareApi.whatsapp(ids, false)) },
    { label: "WhatsApp Web", action: () => sharing(() => shareApi.whatsapp(ids, true)) },
    {
      label: "Post on a website",
      submenu: async () => {
        try {
          const o = await shareApi.options();
          return o.sites.map((s) => ({ label: s.name, action: () => sharing(() => shareApi.site(ids, s.key)) }));
        } catch (e) {
          return [{ label: errText(e), disabled: true }];
        }
      },
    },
    { label: "SmugMug…", action: () => openSmugMug(ids) },
    { separator: true },
    { label: n > 1 ? `Copy ${n} photos` : "Copy photo", shortcut: `${cmd}C`, action: () => copyPhotos(ids) },
    { label: `Show copies to share in ${fileManagerName}`, action: () => sharing(() => shareApi.show(ids)) },
  );
  return items;
}

/** Copy photos to the clipboard, ready to paste into a chat, an email or a website. */
export function copyPhotos(ids: number[]) {
  sharing(() => shareApi.copy(ids));
}
