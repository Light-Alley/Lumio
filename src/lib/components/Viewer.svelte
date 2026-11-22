<script lang="ts">
  import { untrack } from "svelte";
  import { X, ChevronLeft, ChevronRight, Info, CloudOff, ZoomIn, Maximize, RotateCcw, RotateCw, FlipHorizontal2, Expand, Shrink, Play } from "@lucide/svelte";
  import { onDestroy } from "svelte";
  import Stars from "./Stars.svelte";
  import { app, rate, setLabel, saveSetting, rootOnline, selectOnly, trashPhotos, rotatePhotos, startSlideshow, scopeTitle, type Bg } from "../store.svelte";
  import { setFullscreen, toggleFullscreen } from "../fullscreen";
  import { photoUrl, previewUrl, formatDate, formatDuration, labelColor, LABELS, isVideo } from "../api";
  import VideoPlayer from "./VideoPlayer.svelte";
  import { showMenu } from "../menu.svelte";
  import { photoMenu } from "../actions";

  const p = $derived(app.photos[app.viewerIndex]);
  const offline = $derived(p ? !rootOnline(p.root_id) : false);
  const video = $derived(isVideo(p));
  let player: VideoPlayer | undefined = $state();
  let showInfo = $state(true);
  let zoom = $state(1); // 1 = fit
  let pan = $state({ x: 0, y: 0 });
  let dragging = false;
  let dragStart = { x: 0, y: 0, px: 0, py: 0 };
  let loaded = $state(false);
  let failed = $state(false);
  let stage: HTMLDivElement | undefined = $state();

  // Full screen: the bars float over the photo and fade out (with the pointer) when
  // the mouse rests, and come back as soon as it moves.
  let idle = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  /** Full screen was turned on here, so leaving the viewer turns it off again. */
  let ownFullscreen = false;
  function wake() {
    idle = false;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (idle = true), 2500);
  }
  function fullscreen() {
    if (!app.fullscreen) ownFullscreen = true;
    toggleFullscreen();
    wake();
  }
  onDestroy(() => {
    clearTimeout(idleTimer);
    // Starting a slideshow from here keeps the screen for the slideshow.
    if (ownFullscreen && app.fullscreen && !app.slideshow) setFullscreen(false);
  });
  function slideshow() {
    startSlideshow(app.photos, scopeTitle(app.scope), app.viewerIndex);
  }

  const BGS: { id: Bg; label: string; color: string }[] = [
    { id: "black", label: "Black", color: "#000000" },
    { id: "grey", label: "Grey", color: "#6b6b6b" },
    { id: "lightgrey", label: "Light grey", color: "#c8c8c8" },
    { id: "white", label: "White", color: "#ffffff" },
  ];
  const bgColor = $derived(BGS.find((b) => b.id === app.viewerBg)?.color ?? "#000");
  const lightBg = $derived(app.viewerBg === "white" || app.viewerBg === "lightgrey");

  // Lumio refreshes the photo list while it indexes, which hands us a new object for the
  // same photo. Only start over when the photo or its file version really changes;
  // otherwise the sharp image would be hidden again and never come back.
  const pid = $derived(p?.id ?? null);
  const src = $derived(p ? photoUrl(p) : "");

  $effect(() => {
    if (pid === null) return;
    untrack(() => {
      selectOnly(pid);
      zoom = 1;
      pan = { x: 0, y: 0 };
      // Preload neighbours.
      for (const d of [1, -1]) {
        const n = app.photos[app.viewerIndex + d];
        if (n && rootOnline(n.root_id) && !isVideo(n)) new Image().src = photoUrl(n);
      }
    });
  });

  $effect(() => {
    src;
    loaded = false;
    failed = false;
  });

  function close() { app.viewerIndex = -1; }
  function go(d: number) {
    const i = app.viewerIndex + d;
    if (i >= 0 && i < app.photos.length) app.viewerIndex = i;
  }

  function onKey(e: KeyboardEvent) {
    if (app.dialog) return;
    const meta = e.metaKey || e.ctrlKey;
    switch (e.key) {
      case "Escape":
        // The first Esc leaves full screen, the next one closes the viewer.
        if (app.fullscreen) { ownFullscreen = false; setFullscreen(false); } else close();
        break;
      case "f": case "F":
        if (!meta) fullscreen();
        break;
      case "p": case "P":
        if (!meta) { e.preventDefault(); slideshow(); }
        break;
      case "ArrowRight": case "ArrowDown": go(1); break;
      case "ArrowLeft": case "ArrowUp": go(-1); break;
      case " ":
        e.preventDefault();
        if (video && player) player.toggle(); else go(1);
        break;
      case "k": if (video) player?.toggle(); break;
      case "j": if (video) player?.skip(-10); break;
      case "l": if (video) player?.skip(10); break;
      case "m": if (video) player?.toggleMute(); break;
      case "i": showInfo = !showInfo; break;
      case "z": if (!video) toggleZoom(); break;
      case "b": {
        const i = BGS.findIndex((b) => b.id === app.viewerBg);
        setBg(BGS[(i + 1) % BGS.length].id);
        break;
      }
      case "[": case "]":
        if (meta && p && !video) { e.preventDefault(); rotatePhotos([p.id], e.key === "]" ? "right" : "left"); }
        break;
      case "Delete": case "Backspace":
        if (e.key === "Delete" || meta) { e.preventDefault(); if (p) trashPhotos([p.id]); }
        break;
      default:
        if (p && !meta && /^[0-5]$/.test(e.key)) rate([p.id], Number(e.key));
        if (p && !meta && /^[6-9]$/.test(e.key)) {
          const l = LABELS[Number(e.key) - 6].name;
          setLabel([p.id], p.label === l ? "" : l);
        }
    }
    e.stopPropagation();
  }

  function setBg(b: Bg) { app.viewerBg = b; saveSetting("viewer_bg", b); }

  function toggleZoom(e?: MouseEvent) {
    if (video) return;
    if (zoom === 1) {
      zoom = 2.5;
      if (e && stage) {
        const r = stage.getBoundingClientRect();
        pan = { x: (r.width / 2 - (e.clientX - r.left)) * 1.5, y: (r.height / 2 - (e.clientY - r.top)) * 1.5 };
      }
    } else { zoom = 1; pan = { x: 0, y: 0 }; }
  }
  function onWheel(e: WheelEvent) {
    if (video) return;
    e.preventDefault();
    const nz = Math.min(8, Math.max(1, zoom * (e.deltaY < 0 ? 1.12 : 1 / 1.12)));
    if (nz === 1) pan = { x: 0, y: 0 };
    zoom = nz;
  }
  function down(e: MouseEvent) {
    if (zoom === 1 || e.button !== 0) return;
    dragging = true;
    dragStart = { x: e.clientX, y: e.clientY, px: pan.x, py: pan.y };
  }
  function move(e: MouseEvent) {
    if (!dragging) return;
    pan = { x: dragStart.px + e.clientX - dragStart.x, y: dragStart.py + e.clientY - dragStart.y };
  }
</script>

<svelte:window onkeydowncapture={onKey} onmouseup={() => (dragging = false)} onmousemove={(e) => { move(e); wake(); }} />

{#if p}
  <div class="viewer" style:--vbg={bgColor} class:light={lightBg} class:immersive={app.fullscreen} class:idle={app.fullscreen && idle}>
    <div class="top">
      <div class="name">
        <strong>{p.filename}</strong>
        <span class="faint">{app.viewerIndex + 1} / {app.photos.length}</span>
      </div>
      <div class="bgs" title="Background (B)">
        {#each BGS as b}
          <button class="bg" class:on={app.viewerBg === b.id} style:background={b.color} title={b.label} aria-label={`${b.label} background`} onclick={() => setBg(b.id)}></button>
        {/each}
      </div>
      <div class="tools">
        {#if !video}
        <button class="ghost icon" onclick={() => rotatePhotos([p.id], "left")} disabled={offline} title="Rotate left (⌘[ / Ctrl+[)" aria-label="Rotate left"><RotateCcw size={17} /></button>
        <button class="ghost icon" onclick={() => rotatePhotos([p.id], "right")} disabled={offline} title="Rotate right (⌘] / Ctrl+])" aria-label="Rotate right"><RotateCw size={17} /></button>
        <button class="ghost icon" onclick={() => rotatePhotos([p.id], "flip_h")} disabled={offline} title="Flip horizontal" aria-label="Flip horizontal"><FlipHorizontal2 size={17} /></button>
        <button class="ghost icon" onclick={() => toggleZoom()} title="Zoom (Z)">{#if zoom === 1}<ZoomIn size={17} />{:else}<Maximize size={17} />{/if}</button>
        {/if}
        <button class="ghost icon" class:active={showInfo} onclick={() => (showInfo = !showInfo)} title="Info (I)"><Info size={17} /></button>
        <button class="ghost icon" onclick={slideshow} title="Slideshow from this photo (P)" aria-label="Slideshow"><Play size={17} /></button>
        <button class="ghost icon" onclick={fullscreen} title={app.fullscreen ? "Leave full screen (F or Esc)" : "Full screen (F)"} aria-label={app.fullscreen ? "Leave full screen" : "Full screen"}>
          {#if app.fullscreen}<Shrink size={17} />{:else}<Expand size={17} />{/if}
        </button>
        <button class="ghost icon" onclick={close} title="Close (Esc)"><X size={18} /></button>
      </div>
    </div>

    <div
      class="stage"
      bind:this={stage}
      onwheel={onWheel}
      onmousedown={down}
      ondblclick={(e) => toggleZoom(e)}
      oncontextmenu={(e) => showMenu(e, photoMenu(p))}
      role="presentation"
      class:grab={zoom > 1}
    >
      {#if video && !offline}
        <VideoPlayer bind:this={player} {p} autoplay />
      {/if}
      {#if !loaded && p.thumb && !(video && !offline)}
        <img class="ph" src={previewUrl(p)} alt="" draggable="false" />
      {/if}
      {#if !offline && !video}
        {#key p.id}
          <img
            class="main"
            class:hidden={!loaded}
            src={src}
            alt={p.filename}
            draggable="false"
            style:transform={`translate(${pan.x}px, ${pan.y}px) scale(${zoom})`}
            onload={() => (loaded = true)}
            onerror={() => (failed = true)}
          />
        {/key}
      {/if}
      {#if offline}
        <div class="banner"><CloudOff size={14} /> Drive not connected. Showing the saved preview.</div>
      {:else if failed}
        <div class="banner">This file can't be previewed. Try "Open with" from the right-click menu.</div>
      {/if}
      <button class="nav prev" onclick={() => go(-1)} disabled={app.viewerIndex === 0} aria-label="Previous"><ChevronLeft size={28} /></button>
      <button class="nav next" onclick={() => go(1)} disabled={app.viewerIndex >= app.photos.length - 1} aria-label="Next"><ChevronRight size={28} /></button>
    </div>

    {#if showInfo}
      <div class="bottom">
        <Stars value={p.rating} size={16} onchange={(r) => rate([p.id], r)} />
        <div class="labels">
          {#each LABELS as l}
            <button class="lab" class:on={p.label === l.name} style:background={l.color} title={l.name} aria-label={l.name} onclick={() => setLabel([p.id], p.label === l.name ? "" : l.name)}></button>
          {/each}
        </div>
        <span class="meta">{formatDate(p.taken_at)}{#if p.width} · {p.width} × {p.height}{/if}{#if video && p.duration} · {formatDuration(p.duration)}{/if}</span>
        <span class="path faint" title={p.path}>{p.folder}</span>
        {#if labelColor(p.label)}{/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .viewer.immersive .top, .viewer.immersive .bottom { position: absolute; left: 0; right: 0; transition: opacity .25s; }
  .viewer.immersive .top { top: 0; }
  .viewer.immersive .bottom { bottom: 0; }
  .viewer.immersive img.main, .viewer.immersive img.ph { max-width: 100%; max-height: 100%; }
  .viewer.idle .top, .viewer.idle .bottom, .viewer.idle .nav { opacity: 0 !important; pointer-events: none; }
  .viewer.idle { cursor: none; }
  .viewer { position: fixed; inset: 0; z-index: 500; background: var(--vbg); display: flex; flex-direction: column; color: #eee; }
  .top, .bottom {
    display: flex; align-items: center; gap: 14px; padding: 8px 14px; background: rgba(18, 19, 22, 0.88);
    backdrop-filter: blur(10px); z-index: 2;
  }
  .top { justify-content: space-between; }
  .top .name { display: flex; gap: 10px; align-items: baseline; min-width: 0; overflow: hidden; white-space: nowrap; }
  .top button, .bottom button { color: #eee; }
  .top button:hover { background: rgba(255,255,255,.1) !important; }
  .bgs { display: flex; gap: 6px; }
  .bg { width: 20px; height: 20px; border-radius: 50%; padding: 0; border: 2px solid rgba(255,255,255,.25); }
  .bg.on { border-color: var(--accent); box-shadow: 0 0 0 2px rgba(79,140,255,.4); }
  .tools { display: flex; gap: 4px; }
  .stage { flex: 1; position: relative; overflow: hidden; display: flex; align-items: center; justify-content: center; }
  .stage.grab { cursor: grab; }
  img.main, img.ph { max-width: calc(100% - 40px); max-height: calc(100% - 40px); object-fit: contain; transition: transform 0.08s ease-out; }
  img.ph { position: absolute; filter: blur(0.5px); }
  img.hidden { position: absolute; opacity: 0; }
  .light img.main { box-shadow: 0 2px 18px rgba(0,0,0,.18); }
  .banner { position: absolute; top: 14px; left: 50%; transform: translateX(-50%); background: rgba(0,0,0,.72); color: #eee; padding: 6px 12px; border-radius: 14px; display: flex; gap: 6px; align-items: center; }
  .nav { position: absolute; top: 50%; transform: translateY(-50%); background: rgba(0,0,0,.35) !important; border: none; border-radius: 50%; padding: 8px; color: #fff; opacity: 0; transition: opacity .15s; }
  .stage:hover .nav:not(:disabled) { opacity: 1; }
  .nav:hover { background: rgba(0,0,0,.6) !important; }
  .prev { left: 14px; } .next { right: 14px; }
  .bottom { gap: 16px; }
  .labels { display: flex; gap: 5px; }
  .lab { width: 14px; height: 14px; padding: 0; border-radius: 50%; border: 2px solid transparent; opacity: .4; }
  .lab.on { opacity: 1; border-color: #fff; }
  .meta { color: #ccc; }
  .path { margin-left: auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 40%; color: #999; }
</style>
