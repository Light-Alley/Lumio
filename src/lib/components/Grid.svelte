<script lang="ts">
  import { CloudOff, ImageOff, FolderOpen, Plus } from "@lucide/svelte";
  import Stars from "./Stars.svelte";
  import { app, selection, clickSelect, openViewer, rate, setLabel, trashPhotos, rootOnline, addFolderToLibrary, saveSetting, loadPhotos, selectedIds } from "../store.svelte";
  import { thumbUrl, labelColor, LABELS, isMac, type PhotoLite } from "../api";
  import { showMenu } from "../menu.svelte";
  import { photoMenu } from "../actions";

  let scroller: HTMLDivElement | undefined = $state();
  let width = $state(800);
  let height = $state(600);
  let scrollTop = $state(0);

  const GAP = 10;
  const FOOT = 34;
  const cell = $derived(app.thumbSize);
  const cols = $derived(Math.max(1, Math.floor((width - GAP) / (cell + GAP))));
  const colW = $derived((width - GAP) / cols - GAP);
  const rowH = $derived(colW + FOOT + GAP);
  const rows = $derived(Math.ceil(app.photos.length / cols));
  const first = $derived(Math.max(0, Math.floor(scrollTop / rowH) - 2));
  const last = $derived(Math.min(rows, Math.ceil((scrollTop + height) / rowH) + 2));
  const visible = $derived.by(() => {
    const out: { p: PhotoLite; i: number; x: number; y: number }[] = [];
    for (let r = first; r < last; r++) {
      for (let c = 0; c < cols; c++) {
        const i = r * cols + c;
        if (i >= app.photos.length) break;
        out.push({ p: app.photos[i], i, x: GAP + c * (colW + GAP), y: GAP + r * rowH });
      }
    }
    return out;
  });

  // Reset scroll when the set of photos changes scope.
  let lastScope = "";
  $effect(() => {
    const key = JSON.stringify(app.scope);
    if (key !== lastScope && scroller) {
      lastScope = key;
      scroller.scrollTop = 0;
    }
  });

  function ensureVisible(index: number) {
    if (!scroller) return;
    const r = Math.floor(index / cols);
    const top = r * rowH;
    if (top < scroller.scrollTop) scroller.scrollTop = top;
    else if (top + rowH > scroller.scrollTop + height) scroller.scrollTop = top + rowH - height + GAP;
  }

  function onKey(e: KeyboardEvent) {
    if (app.viewerIndex >= 0 || app.dialog || (e.target as HTMLElement)?.closest("input, textarea, select")) return;
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
    const moves: Record<string, number> = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: cols, ArrowUp: -cols, Home: -n, End: n };
    if (e.key in moves) {
      e.preventDefault();
      if (idx < 0) idx = 0;
      else idx = Math.max(0, Math.min(n - 1, idx + moves[e.key]));
      const id = app.photos[idx].id;
      clickSelect(id, e);
      ensureVisible(idx);
    }
  }

  function dragStart(e: DragEvent, p: PhotoLite) {
    if (!selection.has(p.id)) clickSelect(p.id, e as unknown as MouseEvent);
    const ids = selectedIds();
    app.dragIds = ids;
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

  const folderEmptyButHasSubs = $derived(app.scope.kind === "folder" && !app.recursive && app.photos.length === 0 && !app.text && !app.minRating && !app.label);
</script>

<svelte:window onkeydown={onKey} />

<div
  class="scroller"
  bind:this={scroller}
  bind:clientWidth={width}
  bind:clientHeight={height}
  onscroll={() => (scrollTop = scroller!.scrollTop)}
  onmousedown={(e) => { if (e.target === scroller || (e.target as HTMLElement).classList.contains("canvas")) selection.clear(); }}
  role="grid"
  tabindex="0"
>
  <div class="canvas" style:height={`${rows * rowH + GAP}px`}>
    {#each visible as v (v.p.id)}
      {@const p = v.p}
      {@const sel = selection.has(p.id)}
      {@const offline = !rootOnline(p.root_id)}
      <div
        class="cell"
        class:sel
        class:focus={app.focusId === p.id}
        style:transform={`translate(${v.x}px, ${v.y}px)`}
        style:width={`${colW}px`}
        role="gridcell"
        tabindex="-1"
        aria-selected={sel}
        draggable={!offline}
        ondragstart={(e) => dragStart(e, p)}
        onmousedown={(e) => { if (e.button === 0 && !(sel && !e.shiftKey && !e.metaKey && !e.ctrlKey)) clickSelect(p.id, e); }}
        onclick={(e) => { if (sel && !e.shiftKey && !e.metaKey && !e.ctrlKey && selection.size > 1) clickSelect(p.id, e); }}
        ondblclick={() => openViewer(p.id)}
        oncontextmenu={(e) => showMenu(e, photoMenu(p))}
        onkeydown={() => {}}
      >
        <div class="frame" style:height={`${colW}px`}>
          {#if p.thumb}
            <img src={thumbUrl(p)} alt={p.filename} draggable="false" loading="lazy" decoding="async" />
          {:else}
            <div class="nothumb"><ImageOff size={22} /><span>{p.ext.toUpperCase()}</span></div>
          {/if}
          {#if labelColor(p.label)}<span class="label" style:background={labelColor(p.label)}></span>{/if}
          {#if offline}<span class="badge" title="Drive not connected"><CloudOff size={12} /></span>{/if}
          <span class="ext">{p.ext.toUpperCase()}</span>
        </div>
        <div class="foot">
          <span class="fname" title={p.path}>{p.filename}</span>
          <span class="rating" class:has={p.rating > 0}><Stars value={p.rating} size={11} onchange={(r) => rate(selection.has(p.id) ? selectedIds() : [p.id], r)} /></span>
        </div>
      </div>
    {/each}
  </div>

  {#if !app.loading && app.photos.length === 0}
    <div class="empty">
      {#if !app.roots.length}
        <FolderOpen size={44} strokeWidth={1.2} />
        <h2>Welcome to Lumio</h2>
        <p>Add a folder of photos to get started. It can be on this computer, an external drive or a network drive. Lumio keeps small thumbnails so you can still browse them when the drive is unplugged.</p>
        <button class="primary" onclick={() => addFolderToLibrary()}><Plus size={14} /> Add a folder</button>
      {:else if folderEmptyButHasSubs}
        <FolderOpen size={40} strokeWidth={1.2} />
        <p>No photos directly in this folder.</p>
        <button onclick={() => { app.recursive = true; saveSetting("recursive", "1"); loadPhotos(); }}>Show photos from subfolders</button>
      {:else}
        <ImageOff size={40} strokeWidth={1.2} />
        <p>No photos match.</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .scroller { flex: 1; overflow-y: auto; overflow-x: hidden; position: relative; outline: none; }
  .canvas { position: relative; }
  .cell { position: absolute; top: 0; left: 0; border-radius: 8px; padding: 0; contain: layout paint; }
  .frame {
    position: relative; display: flex; align-items: center; justify-content: center;
    background: var(--bg-2); border-radius: 8px; overflow: hidden; border: 2px solid transparent;
  }
  .cell:hover .frame { background: var(--bg-3); }
  .cell.sel .frame { border-color: var(--accent); background: var(--accent-soft); }
  .cell.focus.sel .frame { box-shadow: 0 0 0 1px var(--accent); }
  img { max-width: calc(100% - 12px); max-height: calc(100% - 12px); object-fit: contain; box-shadow: 0 1px 4px rgba(0,0,0,.35); border-radius: 2px; }
  .nothumb { display: flex; flex-direction: column; align-items: center; gap: 4px; color: var(--text-3); font-size: 11px; }
  .label { position: absolute; top: 7px; left: 7px; width: 10px; height: 10px; border-radius: 50%; box-shadow: 0 0 0 2px rgba(0,0,0,.35); }
  .badge { position: absolute; top: 6px; right: 6px; background: rgba(0,0,0,.6); color: #ddd; border-radius: 4px; padding: 2px 3px; display: flex; }
  .ext { position: absolute; bottom: 5px; right: 6px; font-size: 9px; letter-spacing: .04em; background: rgba(0,0,0,.55); color: #ddd; padding: 1px 4px; border-radius: 3px; opacity: 0; transition: opacity .1s; }
  .cell:hover .ext { opacity: 1; }
  .foot { height: 34px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 1px; padding: 0 4px; }
  .fname { font-size: 11.5px; color: var(--text-2); max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cell.sel .fname { color: var(--text); }
  .rating { opacity: 0; height: 14px; }
  .rating.has, .cell:hover .rating { opacity: 1; }
  .empty {
    position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 10px; color: var(--text-2); text-align: center; padding: 40px; pointer-events: none;
  }
  .empty > * { pointer-events: auto; }
  .empty h2 { margin: 6px 0 0; color: var(--text); font-weight: 600; }
  .empty p { max-width: 440px; line-height: 1.5; margin: 0 0 6px; }
</style>
