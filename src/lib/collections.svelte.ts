// Smart collections (saved searches) and opening the duplicate finder.
import { api, type Collection, type RuleSet } from "./api";
import { app, confirm, errorToast, loadCollections, loadPhotos, saveTabs, setScope } from "./store.svelte";

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
