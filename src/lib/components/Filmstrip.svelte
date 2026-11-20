<script lang="ts">
  import { untrack } from "svelte";
  import { ImageOff, Film, Play, CloudOff, RotateCcw, RotateCw, FlipHorizontal2, FlipVertical2 } from "@lucide/svelte";
  import StackBadge from "./StackBadge.svelte";
  import Stars from "./Stars.svelte";
  import EmptyState from "./EmptyState.svelte";
  import { app, selection, clickSelect, openViewer, rate, setLabel, rootOnline, selectedIds, selectOnly, dropOnView, saveSetting, rotatePhotos, isClosedStack, type Bg } from "../store.svelte";
  import { api, thumbUrl, previewUrl, photoUrl, labelColor, LABELS, formatExposure, formatDuration, isVideo, type Photo, type PhotoLite } from "../api";
  import VideoPlayer from "./VideoPlayer.svelte";
  import { showMenu } from "../menu.svelte";
  import { photoMenu } from "../actions";
  import { browseKeys, photoDragStart, photoDragEnd, canDropHere, dragGhosts } from "../browse";

  // ---- the strip
  let strip: HTMLDivElement | undefined = $state();
  let stripW = $state(800);
  let scrollLeft = $state(0);
  let dropHint = $state(false);
  const GAP = 8;
  const ghostCount = $derived(dragGhosts().length);
  const stripH = $derived(Math.max(110, Math.min(220, Math.round(app.thumbSize * 0.75))));
  const tile = $derived(stripH - 48); // image box; the rest is name + stars
  const step = $derived(tile + GAP);
  const first = $derived(Math.max(0, Math.floor(scrollLeft / step) - 3));
  const last = $derived(Math.min(app.photos.length, Math.ceil((scrollLeft + stripW) / step) + 3));

  // ---- the large photo
  const current = $derived(app.photos.find((p) => p.id === app.focusId) ?? null);
  let detail: Photo | null = $state(null);
  let loaded = $state(false);
  let stagePlayer: VideoPlayer | undefined = $state();
  // Don't keep playing underneath the full-screen viewer.
  $effect(() => { if (app.viewerIndex >= 0) stagePlayer?.pause(); });

  // Always show something: focus the first photo when nothing is focused.
  $effect(() => {
    if (!current && app.photos.length && !app.loading) selectOnly(app.photos[0].id);
  });

  // The list is refreshed while Lumio indexes, giving a new object for the same photo.
  // Only reset when the photo or its file version really changes (see Viewer).
  const curId = $derived(current?.id ?? null);
  const bigSrc = $derived(current ? photoUrl(current) : "");

  $effect(() => {
    bigSrc;
    loaded = false;
  });

  $effect(() => {
    const id = curId;
    detail = null;
    if (id === null) return;
    api.getPhoto(id).then((d) => { if (app.focusId === id) detail = d; }).catch(() => {});
    untrack(() => {
      const i = app.photos.findIndex((x) => x.id === id);
      for (const d of [1, -1]) {
        const n = app.photos[i + d];
        if (n && rootOnline(n.root_id) && !isVideo(n)) new Image().src = photoUrl(n);
      }
    });
  });

  let lastScope = "";
  $effect(() => {
    const key = JSON.stringify(app.scope);
    if (key !== lastScope && strip) {
      lastScope = key;
      strip.scrollLeft = 0;
    }
  });

  function reveal(i: number) {
    if (!strip) return;
    const left = i * step;
    if (left < strip.scrollLeft) strip.scrollLeft = left;
    else if (left + step > strip.scrollLeft + stripW) strip.scrollLeft = left + step - stripW + GAP;
  }

  // Keep the focused photo in view when it changes from elsewhere (e.g. switching tabs).
  $effect(() => {
    const i = app.photos.findIndex((p) => p.id === app.focusId);
    if (i >= 0) reveal(i);
  });

  const BGS: { id: Bg; color: string; label: string }[] = [
    { id: "black", color: "#000000", label: "Black" },
    { id: "grey", color: "#6b6b6b", label: "Grey" },
    { id: "lightgrey", color: "#c8c8c8", label: "Light grey" },
    { id: "white", color: "#ffffff", label: "White" },
  ];
  const bg = $derived(BGS.find((b) => b.id === app.viewerBg)?.color ?? "#000");
  const lightBg = $derived(app.viewerBg === "white" || app.viewerBg === "lightgrey");
  function setBg(b: Bg) { app.viewerBg = b; saveSetting("viewer_bg", b); }

  function onKey(e: KeyboardEvent) {
    if (!(e.target as HTMLElement)?.closest("input, textarea, select") && !app.dialog && app.viewerIndex < 0 && e.key === "b" && !e.metaKey && !e.ctrlKey) {
      const i = BGS.findIndex((b) => b.id === app.viewerBg);
      setBg(BGS[(i + 1) % BGS.length].id);
      return;
    }
    browseKeys(e, { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 1, ArrowUp: -1 }, reveal);
  }

  function exif(d: Photo | null): string[] {
    if (!d) return [];
    return [
      d.iso ? `ISO ${d.iso}` : "",
      formatExposure(d.exposure),
      d.fnumber ? `f/${d.fnumber}` : "",
      d.focal ? `${Math.round(d.focal)} mm` : "",
    ].filter(Boolean);
  }

  function wheel(e: WheelEvent) {
    // A vertical mouse wheel scrolls the strip sideways.
    if (strip && Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
      strip.scrollLeft += e.deltaY;
      e.preventDefault();
    }
  }

  function tileClass(p: PhotoLite) { return selection.has(p.id); }
  /** Rotate what's selected, or the photo on the stage. */
  function turnIds() { const ids = selectedIds(); return ids.length ? ids : current ? [current.id] : []; }
</script>

<svelte:window onkeydown={onKey} />

<div class="film">
  <div class="stage" style:background={bg} class:light={lightBg} ondblclick={() => current && openViewer(current.id)} role="img" aria-label={current?.filename ?? ""}>
    {#if current}
      {#if isVideo(current) && rootOnline(current.root_id)}
        <VideoPlayer bind:this={stagePlayer} p={current} compact />
      {:else}
      {#key current.id}
        {#if current.thumb}<img class="ph" src={previewUrl(current)} alt="" draggable="false" class:hide={loaded} />{/if}
        {#if rootOnline(current.root_id)}
          <img class="big" src={bigSrc} alt={current.filename} draggable="false" onload={() => (loaded = true)} class:show={loaded} />
        {:else}
          <span class="off"><CloudOff size={14} /> Drive not connected, showing the saved preview</span>
        {/if}
      {/key}
      {/if}
    {:else}
      <EmptyState />
    {/if}
    <div class="bgs">
      {#each BGS as b}
        <button class="bgdot" class:on={app.viewerBg === b.id} style:background={b.color} title={`${b.label} background (B)`} aria-label={b.label} onclick={(e) => { e.stopPropagation(); setBg(b.id); }}></button>
      {/each}
    </div>
  </div>

  <div class="info">
    <span class="exif">{#each exif(detail) as x}<span>{x}</span>{/each}</span>
    <span class="name">{current?.filename ?? ""}</span>
    <span class="right">
      {#if current}
        {#if !isVideo(current)}
        <span class="turns">
          <button class="ghost icon" title="Rotate left (⌘[ / Ctrl+[)" aria-label="Rotate left" onclick={() => rotatePhotos(turnIds(), "left")}><RotateCcw size={14} /></button>
          <button class="ghost icon" title="Rotate right (⌘] / Ctrl+])" aria-label="Rotate right" onclick={() => rotatePhotos(turnIds(), "right")}><RotateCw size={14} /></button>
          <button class="ghost icon" title="Flip horizontal" aria-label="Flip horizontal" onclick={() => rotatePhotos(turnIds(), "flip_h")}><FlipHorizontal2 size={14} /></button>
          <button class="ghost icon" title="Flip vertical" aria-label="Flip vertical" onclick={() => rotatePhotos(turnIds(), "flip_v")}><FlipVertical2 size={14} /></button>
        </span>
        {/if}
        {#each LABELS as l, i}
          <button class="sw" class:on={current.label === l.name} style:--c={l.color} title={`${l.name} (${i + 6})`} aria-label={l.name} onclick={() => setLabel([current.id], current.label === l.name ? "" : l.name)}></button>
        {/each}
        <Stars value={current.rating} size={14} onchange={(r) => rate([current.id], r)} />
      {/if}
    </span>
  </div>

  <div
    class="strip"
    class:drop={dropHint}
    style:height={`${stripH}px`}
    bind:this={strip}
    bind:clientWidth={stripW}
    onscroll={() => (scrollLeft = strip!.scrollLeft)}
    onwheel={wheel}
    ondragover={(e) => { if (canDropHere(e)) { e.preventDefault(); dropHint = true; } }}
    ondragleave={() => (dropHint = false)}
    ondrop={(e) => { e.preventDefault(); dropHint = false; dropOnView(e); }}
    role="listbox"
    tabindex="0"
  >
    <div class="track" style:width={`${app.photos.length * step + GAP}px`}>
      {#each [...dragGhosts(), ...app.photos.slice(first, last)] as p, k (p.id)}
        {@const ghost = k < ghostCount}
        {@const i = ghost ? -200 : first + k - ghostCount}
        {@const sel = tileClass(p)}
        <div
          class="tile"
          class:sel
          class:focus={app.focusId === p.id}
          class:ghost
          class:member={p.stack_id != null && !isClosedStack(p)}
          style:transform={`translateX(${GAP + i * step}px)`}
          style:width={`${tile}px`}
          role="option"
          tabindex="-1"
          aria-selected={sel}
          draggable={rootOnline(p.root_id)}
          ondragstart={(e) => photoDragStart(e, p)}
          ondragend={photoDragEnd}
          onmousedown={(e) => { if (e.button === 0 && !(sel && !e.shiftKey && !e.metaKey && !e.ctrlKey)) clickSelect(p.id, e); }}
          onclick={(e) => { if (sel && !e.shiftKey && !e.metaKey && !e.ctrlKey) clickSelect(p.id, e); }}
          ondblclick={() => openViewer(p.id)}
          oncontextmenu={(e) => showMenu(e, photoMenu(p))}
          onkeydown={() => {}}
        >
          <div class="box" style:height={`${tile}px`}>
            {#if p.thumb}<img src={thumbUrl(p)} alt={p.filename} draggable="false" loading="lazy" />{:else if isVideo(p)}<Film size={18} />{:else}<ImageOff size={18} />{/if}
            {#if isVideo(p)}<span class="vid"><Play size={8} fill="currentColor" />{formatDuration(p.duration)}</span>{/if}
            {#if labelColor(p.label)}<span class="lab" style:background={labelColor(p.label)}></span>{/if}
            {#if p.stack_id != null}<span class="stk"><StackBadge {p} small /></span>{/if}
          </div>
          <div class="cap">
            <span class="fn" title={p.path}>{p.filename}</span>
            <span class="st" class:has={p.rating > 0}><Stars value={p.rating} size={10} onchange={(r) => rate(selection.has(p.id) ? selectedIds() : [p.id], r)} /></span>
          </div>
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .film { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .stage { flex: 1; min-height: 120px; position: relative; display: flex; align-items: center; justify-content: center; overflow: hidden; }
  .stage img { position: absolute; max-width: calc(100% - 40px); max-height: calc(100% - 40px); object-fit: contain; }
  .ph { filter: blur(0.5px); width: calc(100% - 40px); height: calc(100% - 40px); }
  .ph.hide { visibility: hidden; }
  .big { opacity: 0; transition: opacity .12s; box-shadow: 0 2px 14px rgba(0,0,0,.35); }
  .big.show { opacity: 1; }
  .off { position: absolute; bottom: 10px; left: 50%; transform: translateX(-50%); display: flex; align-items: center; gap: 6px; background: rgba(0,0,0,.6); color: #ddd; padding: 4px 10px; border-radius: 12px; font-size: 11.5px; }
  .bgs { position: absolute; top: 10px; right: 12px; display: flex; gap: 6px; opacity: 0; transition: opacity .15s; }
  .stage:hover .bgs { opacity: 1; }
  .bgdot { width: 14px; height: 14px; padding: 0; border-radius: 50%; border: 1px solid rgba(128,128,128,.7); }
  .bgdot.on { box-shadow: 0 0 0 2px var(--accent); }
  .info { height: 34px; flex-shrink: 0; display: grid; grid-template-columns: 1fr auto 1fr; align-items: center; padding: 0 14px; background: var(--bg-2); border-top: 1px solid var(--line); border-bottom: 1px solid var(--line); font-size: 12.5px; color: var(--text-2); }
  .exif { display: flex; gap: 16px; white-space: nowrap; overflow: hidden; }
  .name { color: var(--text); font-weight: 500; text-align: center; }
  .right { display: flex; justify-content: flex-end; align-items: center; gap: 5px; }
  .sw { width: 12px; height: 12px; padding: 0; border-radius: 50%; background: var(--c) !important; border: 2px solid transparent; opacity: .35; }
  .sw.on { opacity: 1; border-color: var(--text); }
  .right :global(.stars) { margin-left: 8px; }
  .turns { display: flex; gap: 1px; margin-right: 10px; }
  .turns .icon { padding: 3px; }
  .stk { position: absolute; bottom: 4px; left: 4px; }
  .vid { position: absolute; bottom: 4px; right: 4px; display: flex; align-items: center; gap: 2px; font-size: 9px; font-variant-numeric: tabular-nums; background: rgba(0,0,0,.6); color: #eee; padding: 1px 4px; border-radius: 3px; }
  .tile.member .box { background: color-mix(in srgb, var(--accent) 9%, var(--bg-2)); }
  .strip { flex-shrink: 0; overflow-x: auto; overflow-y: hidden; position: relative; outline: none; background: var(--bg); }
  .strip.drop { box-shadow: inset 0 0 0 2px var(--accent); background: var(--accent-soft); }
  .track { position: relative; height: 100%; }
  .ghost { visibility: hidden; pointer-events: none; }
  .tile { position: absolute; top: 6px; left: 0; contain: layout paint; }
  .box { display: flex; align-items: center; justify-content: center; background: var(--bg-2); border: 2px solid transparent; border-radius: 6px; overflow: hidden; position: relative; color: var(--text-3); }
  .tile:hover .box { background: var(--bg-3); }
  .tile.sel .box { border-color: var(--accent); background: var(--accent-soft); }
  .tile.focus .box { box-shadow: 0 0 0 1px var(--accent); }
  .box img { max-width: calc(100% - 8px); max-height: calc(100% - 8px); object-fit: contain; border-radius: 2px; }
  .lab { position: absolute; top: 5px; left: 5px; width: 9px; height: 9px; border-radius: 50%; box-shadow: 0 0 0 2px rgba(0,0,0,.35); }
  .cap { height: 30px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 1px; }
  .fn { font-size: 11px; color: var(--text-2); max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tile.sel .fn { color: var(--text); }
  .st { opacity: 0; height: 12px; }
  .st.has, .tile:hover .st { opacity: 1; }
</style>
