<script lang="ts">
  import { Search, LayoutGrid, List, GalleryHorizontal, Map as MapIcon, Info, Sparkles, Settings, ArrowDownWideNarrow, ArrowUpNarrowWide, FolderTree, X, Share2 } from "@lucide/svelte";
  import Stars from "./Stars.svelte";
  import IdentityPlate from "./IdentityPlate.svelte";
  import { app, loadPhotos, saveSetting, setView, scopeTitle, selection, selectedIds } from "../store.svelte";
  import { showMenu } from "../menu.svelte";
  import { shareMenu } from "../actions";
  import { LABELS } from "../api";

  let searchTimer: ReturnType<typeof setTimeout>;
  function onSearch() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(loadPhotos, 250);
  }

  const title = $derived(scopeTitle(app.scope));

  function openShare(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    // Open the menu under the button rather than at the pointer.
    showMenu({ preventDefault() {}, clientX: r.left, clientY: r.bottom + 4 } as MouseEvent, shareMenu(selectedIds()));
  }
</script>

<svelte:window onkeydown={(e) => {
  if (e.metaKey || e.ctrlKey || e.altKey || app.dialog || app.viewerIndex >= 0 || app.showSettings || (e.target as HTMLElement)?.closest("input, textarea, select")) return;
  const v = ({ g: "grid", l: "list", f: "filmstrip", m: "map" } as const)[e.key.toLowerCase() as "g"];
  if (v) setView(v);
}} />

<header class="toolbar" data-tauri-drag-region>
  <IdentityPlate />
  <div class="title" data-tauri-drag-region>
    <h1 data-tauri-drag-region>{title}</h1>
    {#if app.scope.kind === "folder"}
      <button class="ghost icon" class:active={app.recursive} title="Include photos in subfolders" onclick={() => { app.recursive = !app.recursive; saveSetting("recursive", app.recursive ? "1" : "0"); loadPhotos(); }}>
        <FolderTree size={15} />
      </button>
    {/if}
  </div>

  <div class="search">
    <Search size={14} />
    <input type="search" placeholder="Search names, keywords, captions, places…" bind:value={app.text} oninput={onSearch} />
    {#if app.text}<button class="ghost icon clear" onclick={() => { app.text = ""; loadPhotos(); }} aria-label="Clear"><X size={13} /></button>{/if}
  </div>

  <div class="filters">
    <span class="faint">Min.</span>
    <Stars value={app.minRating} size={13} onchange={(v) => { app.minRating = v; loadPhotos(); }} />
    <div class="labels">
      {#each LABELS as l}
        <button class="swatch" class:on={app.label === l.name} style:--c={l.color} title={`Only ${l.name}`} onclick={() => { app.label = app.label === l.name ? "" : l.name; loadPhotos(); }} aria-label={l.name}></button>
      {/each}
    </div>
    <select bind:value={app.sort} onchange={loadPhotos} title="Sort by">
      {#if app.scope.kind === "ai"}<option value="relevance">Best match</option>{/if}
      <option value="date">Date taken</option>
      <option value="name">Name</option>
      <option value="rating">Rating</option>
      <option value="modified">Date modified</option>
      <option value="size">File size</option>
    </select>
    <button class="ghost icon" title={app.desc ? "Descending" : "Ascending"} onclick={() => { app.desc = !app.desc; loadPhotos(); }}>
      {#if app.desc}<ArrowDownWideNarrow size={15} />{:else}<ArrowUpNarrowWide size={15} />{/if}
    </button>
  </div>

  <div class="right">
    <input class="size" type="range" min="100" max="360" step="10" bind:value={app.thumbSize} onchange={() => saveSetting("thumb_size", String(app.thumbSize))} title="Thumbnail size" />
    <div class="seg">
      <button class:active={app.view === "grid"} onclick={() => setView("grid")} title="Grid (G)"><LayoutGrid size={15} /></button>
      <button class:active={app.view === "list"} onclick={() => setView("list")} title="List (L)"><List size={15} /></button>
      <button class:active={app.view === "filmstrip"} onclick={() => setView("filmstrip")} title="Large photo with filmstrip (F)"><GalleryHorizontal size={15} /></button>
      <button class:active={app.view === "map"} onclick={() => setView("map")} title="Map (M)"><MapIcon size={15} /></button>
    </div>
    <div class="seg">
      <button class:active={app.panel === "info"} onclick={() => (app.panel = app.panel === "info" ? null : "info")} title="Info & metadata"><Info size={15} /></button>
      <button class:active={app.panel === "ai"} onclick={() => (app.panel = app.panel === "ai" ? null : "ai")} title="Ask AI"><Sparkles size={15} /> Ask AI</button>
    </div>
    <button class="ghost icon" disabled={selection.size === 0} onclick={openShare} title={selection.size ? "Share the selected photos" : "Select photos to share"}><Share2 size={16} /></button>
    <button class="ghost icon" onclick={() => (app.showSettings = true)} title="Settings"><Settings size={16} /></button>
  </div>
</header>

<style>
  .toolbar {
    height: 50px; flex-shrink: 0; display: flex; align-items: center; gap: 14px; padding: 0 12px;
    background: var(--bg-2); border-bottom: 1px solid var(--line);
  }
  .title { display: flex; align-items: center; gap: 6px; min-width: 120px; max-width: 260px; }
  h1 { font-size: 15px; font-weight: 600; margin: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .search { flex: 1; max-width: 420px; position: relative; display: flex; align-items: center; }
  .search > :global(svg) { position: absolute; left: 9px; color: var(--text-3); }
  .search input { padding-left: 28px; height: 30px; border-radius: 15px; background: var(--bg); }
  .clear { position: absolute; right: 4px; padding: 3px; }
  .filters { display: flex; align-items: center; gap: 8px; }
  .filters select { width: auto; height: 28px; padding: 2px 6px; }
  .labels { display: flex; gap: 4px; }
  .swatch { width: 14px; height: 14px; padding: 0; border-radius: 50%; background: var(--c) !important; border: 2px solid transparent; opacity: 0.45; }
  .swatch.on { opacity: 1; border-color: var(--text); }
  .right { margin-left: auto; display: flex; align-items: center; gap: 8px; }
  .size { width: 90px; accent-color: var(--accent); }
  .seg { display: flex; }
  .seg button { border-radius: 0; margin-left: -1px; height: 28px; }
  .seg button:first-child { border-radius: 6px 0 0 6px; }
  .seg button:last-child { border-radius: 0 6px 6px 0; }
  @media (max-width: 1250px) { .labels, .filters .faint { display: none; } }
</style>
