<script lang="ts">
  import { CloudOff, ImageOff, Film, Play, Clock } from "@lucide/svelte";
  import StackBadge from "./StackBadge.svelte";
  import Stars from "./Stars.svelte";
  import EmptyState from "./EmptyState.svelte";
  import { app, selection, clickSelect, openViewer, rate, rootOnline, selectedIds, dropOnView, isClosedStack } from "../store.svelte";
  import { thumbUrl, previewUrl, labelColor, isVideo, formatDuration, type PhotoLite } from "../api";
  import { browseKeys, photoDragStart, photoDragEnd, canDropHere, dragGhosts } from "../browse";
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
  // Big tiles on a sharp (Retina) screen need more pixels than the 480 px thumbnail has:
  // use the saved offline preview when there is one (it falls back to the thumbnail).
  const sharp = $derived(colW * (typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1) > 560);
  const rows = $derived(Math.ceil(app.photos.length / cols));
  const first = $derived(Math.max(0, Math.floor(scrollTop / rowH) - 2));
  const last = $derived(Math.min(rows, Math.ceil((scrollTop + height) / rowH) + 2));
  const visible = $derived.by(() => {
    const out: { p: PhotoLite; i: number; x: number; y: number; ghost?: boolean }[] = [];
    for (const p of dragGhosts()) out.push({ p, i: -1, x: -9999, y: 0, ghost: true });
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
    browseKeys(e, { ArrowRight: 1, ArrowLeft: -1, ArrowDown: cols, ArrowUp: -cols }, ensureVisible);
  }

  let dropHint = $state(false);
</script>

<svelte:window onkeydown={onKey} />

<div
  class="scroller"
  bind:this={scroller}
  bind:clientWidth={width}
  bind:clientHeight={height}
  onscroll={() => (scrollTop = scroller!.scrollTop)}
  onmousedown={(e) => { if (e.target === scroller || (e.target as HTMLElement).classList.contains("canvas")) selection.clear(); }}
  ondragover={(e) => { if (canDropHere(e)) { e.preventDefault(); dropHint = true; } }}
  ondragleave={() => (dropHint = false)}
  ondrop={(e) => { e.preventDefault(); dropHint = false; dropOnView(e); }}
  class:drop={dropHint}
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
        class:ghost={v.ghost}
        class:member={p.stack_id != null && !isClosedStack(p)}
        style:transform={`translate(${v.x}px, ${v.y}px)`}
        style:width={`${colW}px`}
        role="gridcell"
        tabindex="-1"
        aria-selected={sel}
        draggable="true"
        ondragstart={(e) => photoDragStart(e, p)}
        ondragend={photoDragEnd}
        onmousedown={(e) => { if (e.button === 0 && !(sel && !e.shiftKey && !e.metaKey && !e.ctrlKey)) clickSelect(p.id, e); }}
        onclick={(e) => { if (sel && !e.shiftKey && !e.metaKey && !e.ctrlKey && selection.size > 1) clickSelect(p.id, e); }}
        ondblclick={() => openViewer(p.id)}
        oncontextmenu={(e) => showMenu(e, photoMenu(p))}
        onkeydown={() => {}}
      >
        <div class="frame" style:height={`${colW}px`}>
          {#if p.thumb}
            <img src={sharp ? previewUrl(p) : thumbUrl(p)} alt={p.filename} draggable="false" loading="lazy" decoding="async" />
          {:else}
            <div class="nothumb">{#if isVideo(p)}<Film size={22} />{:else}<ImageOff size={22} />{/if}<span>{p.ext.toUpperCase()}</span></div>
          {/if}
          {#if labelColor(p.label)}<span class="label" style:background={labelColor(p.label)}></span>{/if}
          {#if p.pending}<span class="badge" title="Moved while a drive was away. The file follows when it's connected."><Clock size={12} /></span>
          {:else if offline}<span class="badge" title="Drive not connected"><CloudOff size={12} /></span>{/if}
          {#if isVideo(p)}
            <span class="vid" title="Video"><Play size={9} fill="currentColor" />{formatDuration(p.duration)}</span>
          {:else}
            <span class="ext">{p.ext.toUpperCase()}</span>
          {/if}
          {#if p.stack_id != null}<span class="stackpos"><StackBadge {p} /></span>{/if}
        </div>
        <div class="foot">
          <span class="fname" title={p.path}>{p.filename}</span>
          <span class="rating" class:has={p.rating > 0}><Stars value={p.rating} size={11} onchange={(r) => rate(selection.has(p.id) ? selectedIds() : [p.id], r)} /></span>
        </div>
      </div>
    {/each}
  </div>

  <EmptyState />
</div>

<style>
  .scroller { flex: 1; overflow-y: auto; overflow-x: hidden; position: relative; outline: none; }
  .canvas { position: relative; }
  .ghost { visibility: hidden; pointer-events: none; }
  .scroller.drop { box-shadow: inset 0 0 0 2px var(--accent); background: var(--accent-soft); }
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
  .vid { position: absolute; bottom: 5px; right: 6px; display: flex; align-items: center; gap: 3px; font-size: 10px; font-variant-numeric: tabular-nums; background: rgba(0,0,0,.6); color: #eee; padding: 1px 5px; border-radius: 3px; }
  .stackpos { position: absolute; bottom: 5px; left: 6px; }
  .cell.member .frame { background: color-mix(in srgb, var(--accent) 9%, var(--bg-2)); }
  .foot { height: 34px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 1px; padding: 0 4px; }
  .fname { font-size: 11.5px; color: var(--text-2); max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cell.sel .fname { color: var(--text); }
  .rating { opacity: 0; height: 14px; }
  .rating.has, .cell:hover .rating { opacity: 1; }
</style>
