<script lang="ts">
  import { Plus, X, Folder, Images, Clock, Star, MapPin, MapPinOff, Sparkles, ListFilter } from "@lucide/svelte";
  import { app, switchTab, newTab, closeTab, moveTab, dropOnTab, scopeTitle, type Tab } from "../store.svelte";
  import { isPhotoDrag } from "../browse";

  const ICONS = { all: Images, recent: Clock, rated: Star, geotagged: MapPin, nogps: MapPinOff, folder: Folder, collection: ListFilter, ai: Sparkles };

  // Hovering a tab while dragging photos opens it, so you can drop into its folder or onto a
  // subfolder in the sidebar. Dropping straight onto the tab moves the photos into its folder.
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  let dropOn = $state(-1);
  let tabDrag = -1;

  function enter(e: DragEvent, i: number) {
    if (isPhotoDrag(e)) {
      e.preventDefault();
      if (dropOn !== i) {
        dropOn = i;
        clearTimeout(hoverTimer);
        if (i !== app.activeTab) hoverTimer = setTimeout(() => switchTab(i), 700);
      }
    } else if (tabDrag >= 0) {
      e.preventDefault();
    }
  }
  function leave(e: DragEvent, i: number) {
    // Moving over the tab's own icon or text also fires dragleave; ignore that.
    const to = e.relatedTarget as Node | null;
    if (to && (e.currentTarget as HTMLElement).contains(to)) return;
    if (dropOn === i) dropOn = -1;
    clearTimeout(hoverTimer);
  }
  function drop(e: DragEvent, i: number) {
    e.preventDefault();
    clearTimeout(hoverTimer);
    dropOn = -1;
    if (tabDrag >= 0) {
      moveTab(tabDrag, i);
      tabDrag = -1;
    } else if (isPhotoDrag(e)) {
      dropOnTab(i, e);
    }
  }
  function title(t: Tab) {
    return scopeTitle(t.scope);
  }
  function tip(t: Tab) {
    return t.scope.kind === "folder" ? t.scope.path : scopeTitle(t.scope);
  }
</script>

<div class="tabs" role="tablist">
  {#each app.tabs as t, i (t.id)}
    {@const Icon = ICONS[t.scope.kind]}
    <div
      class="tab"
      class:active={i === app.activeTab}
      class:drop={dropOn === i}
      role="tab"
      tabindex="0"
      aria-selected={i === app.activeTab}
      title={tip(t)}
      draggable="true"
      ondragstart={(e) => { tabDrag = i; e.dataTransfer?.setData("application/x-lumio-tab", String(i)); }}
      ondragend={() => (tabDrag = -1)}
      ondragenter={(e) => enter(e, i)}
      ondragover={(e) => enter(e, i)}
      ondragleave={(e) => leave(e, i)}
      ondrop={(e) => drop(e, i)}
      onmousedown={(e) => { if (e.button === 1) { e.preventDefault(); closeTab(i); } else if (e.button === 0) switchTab(i); }}
      onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") switchTab(i); }}
    >
      <Icon size={13} />
      <span class="t">{title(i === app.activeTab ? { ...t, scope: app.scope } : t)}</span>
      {#if app.tabs.length > 1}
        <button class="x" aria-label="Close tab" title="Close tab" onmousedown={(e) => e.stopPropagation()} onclick={(e) => { e.stopPropagation(); closeTab(i); }}><X size={12} /></button>
      {/if}
    </div>
  {/each}
  <button class="add" title="New tab (⌘T / Ctrl+T)" aria-label="New tab" onclick={() => newTab()}><Plus size={14} /></button>
</div>

<svelte:window
  onkeydown={(e) => {
    const meta = e.metaKey || e.ctrlKey;
    if (!meta || app.dialog) return;
    const k = e.key.toLowerCase();
    if (k === "t") { e.preventDefault(); newTab(); }
    else if (k === "w" && app.tabs.length > 1) { e.preventDefault(); closeTab(app.activeTab); }
    else if (/^[1-9]$/.test(e.key)) { e.preventDefault(); switchTab(Math.min(Number(e.key), app.tabs.length) - 1); }
  }}
/>

<style>
  .tabs { height: 34px; flex-shrink: 0; display: flex; align-items: flex-end; gap: 2px; padding: 0 8px; background: var(--bg-2); border-bottom: 1px solid var(--line); overflow-x: auto; overflow-y: hidden; scrollbar-width: none; }
  .tab {
    height: 28px; min-width: 110px; max-width: 220px; display: flex; align-items: center; gap: 6px; padding: 0 6px 0 10px;
    border: 1px solid transparent; border-bottom: none; border-radius: 7px 7px 0 0; color: var(--text-2); font-size: 12px;
    flex-shrink: 1; position: relative; top: 1px;
  }
  .tab:hover { background: var(--bg-3); }
  .tab.active { background: var(--bg); color: var(--text); border-color: var(--line); }
  .tab.drop { background: var(--accent-soft); color: var(--text); box-shadow: inset 0 0 0 1px var(--accent); }
  .tab :global(svg) { flex-shrink: 0; opacity: .8; }
  .t { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .x { padding: 2px; border: none; background: none; border-radius: 4px; opacity: 0; color: var(--text-2); }
  .tab:hover .x, .tab.active .x { opacity: 1; }
  .x:hover { background: var(--bg-4); color: var(--text); }
  .add { align-self: center; padding: 3px; border: none; background: none; color: var(--text-2); margin-left: 2px; }
  .add:hover { background: var(--bg-3); color: var(--text); }
</style>
