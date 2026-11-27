// "Is there a newer Frameloft?" A quiet check soon after launch (and every few hours while
// Frameloft stays open), plus Frameloft > Check for Updates… (Help menu on Windows and Linux).
// The check asks GitHub for the latest published release; see src-tauri/src/updates.rs.
import { listen } from "@tauri-apps/api/event";
import { api, errText, type UpdateInfo } from "./api";
import { app, saveSetting, toast } from "./store.svelte";

export const updates = $state({
  info: null as UpdateInfo | null,
  /** The "new version" window is showing. */
  open: false,
  checking: false,
  installing: false,
  downloaded: 0,
  total: null as number | null,
});

/** Automatic checks are on unless switched off in Settings. */
export function autoCheckOn() {
  return (app.settings.update_check ?? "1") !== "0";
}

const QUIET_GAP = 12 * 60 * 60 * 1000;

/**
 * Ask GitHub whether a newer release is out. `manual` is the menu item: it always asks,
 * and says so when Frameloft is up to date or the check fails. The automatic check stays
 * silent unless there is something new (and not a version the user chose to skip).
 */
export async function checkForUpdates(manual: boolean) {
  if (updates.checking || updates.installing) return;
  if (!manual) {
    if (!autoCheckOn()) return;
    const last = Number(app.settings.update_last_check || 0);
    if (Date.now() - last < QUIET_GAP) return;
  }
  updates.checking = true;
  const slow = manual ? setTimeout(() => toast("Checking for updates…"), 500) : undefined;
  try {
    const info = await api.checkForUpdates();
    saveSetting("update_last_check", String(Date.now()));
    if (info.newer) {
      updates.info = info;
      if (manual) updates.open = true;
      else if (app.settings.update_skip !== info.latest) {
        toast(`Frameloft ${info.latest} is available. You have ${info.current}.`, "info", { label: "See what's new", run: () => (updates.open = true) }, 20000);
      }
    } else if (manual) {
      toast(`You have the latest version of Frameloft (${info.current}).`, "success");
    }
  } catch (e) {
    if (manual) toast(errText(e), "error");
    else console.info("update check:", errText(e));
  } finally {
    clearTimeout(slow);
    updates.checking = false;
  }
}

let started = false;
/** First check a little after launch, so it never slows down opening the library. */
export function startUpdateChecks() {
  if (started) return;
  started = true;
  setTimeout(() => checkForUpdates(false), 8000);
  setInterval(() => checkForUpdates(false), 60 * 60 * 1000);
}

export async function installUpdate() {
  if (!updates.info?.can_install || updates.installing) return;
  updates.installing = true;
  updates.downloaded = 0;
  updates.total = null;
  const stop = await listen<{ downloaded: number; total: number | null }>("update-progress", (e) => {
    updates.downloaded = e.payload.downloaded;
    updates.total = e.payload.total;
  });
  try {
    await api.installUpdate(); // Frameloft restarts when this succeeds.
  } catch (e) {
    toast(errText(e), "error", { label: "Open release page", run: () => openReleasePage() }, 15000);
  } finally {
    stop();
    updates.installing = false;
  }
}

export function openReleasePage() {
  api.openProjectPage(updates.info?.url).catch((e) => toast(errText(e), "error"));
}

export function skipThisVersion() {
  if (updates.info) saveSetting("update_skip", updates.info.latest);
  updates.open = false;
}
