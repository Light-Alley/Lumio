<script lang="ts">
  // Full-screen slideshow of a list of photos (a selection, a folder or a group).
  // Each photo fades in over the last one once it has loaded, so a slow drive never
  // shows a half-drawn picture. Videos play to the end and then the show moves on.
  import { onMount, onDestroy } from "svelte";
  import { X, ChevronLeft, ChevronRight, Play, Pause, Shuffle, Expand, Shrink, CloudOff } from "@lucide/svelte";
  import { app, rootOnline, saveSetting, selectOnly, openViewer } from "../store.svelte";
  import { photoUrl, thumbUrl, previewUrl, isVideo, formatDate, type PhotoLite } from "../api";
  import { setFullscreen, toggleFullscreen } from "../fullscreen";

  const SPEEDS = [2, 3, 5, 8, 10, 15, 30];

  const show = app.slideshow!;
  const photos: PhotoLite[] = show.photos;
  /** The playing order: indexes into `photos`. */
  let order = $state(photos.map((_, i) => i));
  let pos = $state(0);
  let playing = $state(true);
  let shuffle = $state(app.settings.slideshow_shuffle === "1");
  let seconds = $state(SPEEDS.includes(Number(app.settings.slideshow_seconds)) ? Number(app.settings.slideshow_seconds) : 5);
  let showCaption = $state(app.settings.slideshow_caption !== "0");

  interface Layer { key: number; p: PhotoLite; ready: boolean; offline: boolean }
  let layers = $state<Layer[]>([]);
  let seq = 0;
  const current = $derived(layers[layers.length - 1]);

  let idle = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let advanceTimer: ReturnType<typeof setTimeout> | undefined;
  /** Whether the window was already full screen, so ending the show leaves it as it was. */
  let wasFullscreen = app.fullscreen;

  const bg = $derived(({ black: "#000", grey: "#6b6b6b", lightgrey: "#c8c8c8", white: "#fff" } as const)[app.viewerBg] ?? "#000");

  function shuffled(first: number): number[] {
    const rest = photos.map((_, i) => i).filter((i) => i !== first);
    for (let i = rest.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [rest[i], rest[j]] = [rest[j], rest[i]];
    }
    return [first, ...rest];
  }

  function display() {
    clearTimeout(advanceTimer);
    const p = photos[order[pos]];
    const offline = !rootOnline(p.root_id);
    // Offline photos have no file to load, only the saved thumbnail.
    const layer: Layer = { key: seq++, p, ready: false, offline };
    layers = [...layers.slice(-1), layer];
    if (offline && !p.thumb) queueMicrotask(() => loaded(layer));
    // Warm the cache for the next photo.
    const n = photos[order[(pos + 1) % order.length]];
    if (n && rootOnline(n.root_id) && !isVideo(n)) new Image().src = photoUrl(n);
  }

  function loaded(l: Layer) {
    const live = layers.find((x) => x.key === l.key);
    if (!live) return;
    live.ready = true;
    // Drop the photo underneath once the fade has finished.
    setTimeout(() => { layers = layers.filter((x) => x.key >= l.key); }, 900);
    schedule();
  }

  function schedule() {
    clearTimeout(advanceTimer);
    if (!playing || !current?.ready) return;
    if (isVideo(current.p) && !current.offline) return; // moves on when the video ends
    advanceTimer = setTimeout(() => go(1), seconds * 1000);
  }

  function go(d: number) {
    if (photos.length < 2 && d !== 0) return;
    pos = (pos + d + order.length) % order.length;
    display();
  }

  function togglePlay() {
    playing = !playing;
    if (playing) schedule(); else clearTimeout(advanceTimer);
    wake();
  }
  function setSeconds(s: number) {
    seconds = s;
    saveSetting("slideshow_seconds", String(s));
    schedule();
  }
  function toggleShuffle() {
    shuffle = !shuffle;
    saveSetting("slideshow_shuffle", shuffle ? "1" : "0");
    const at = order[pos];
    order = shuffle ? shuffled(at) : photos.map((_, i) => i);
    pos = order.indexOf(at);
  }

  function close() {
    // Leave the last photo shown selected, so the grid opens where the show ended.
    if (current) selectOnly(current.p.id);
    if (show.backToViewer && current) openViewer(current.p.id);
    app.slideshow = null;
  }

  function wake() {
    idle = false;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (idle = true), 2500);
  }

  function onKey(e: KeyboardEvent) {
    if (app.dialog) return;
    const meta = e.metaKey || e.ctrlKey;
    switch (e.key) {
      case "Escape": close(); break;
      case " ": case "k": e.preventDefault(); togglePlay(); break;
      case "ArrowRight": case "ArrowDown": go(1); wake(); break;
      case "ArrowLeft": case "ArrowUp": go(-1); wake(); break;
      case "f": case "F": if (!meta) toggleFullscreen(); break;
      case "s": case "S": if (!meta) toggleShuffle(); break;
      case "i": case "I":
        showCaption = !showCaption;
        saveSetting("slideshow_caption", showCaption ? "1" : "0");
        break;
      case "+": case "=": setSeconds(SPEEDS[Math.min(SPEEDS.length - 1, SPEEDS.indexOf(seconds) + 1)]); wake(); break;
      case "-": case "_": setSeconds(SPEEDS[Math.max(0, SPEEDS.indexOf(seconds) - 1)]); wake(); break;
      default: return;
    }
    e.preventDefault();
    e.stopPropagation();
  }

  onMount(() => {
    if (shuffle) order = shuffled(show.start);
    pos = order.indexOf(show.start);
    display();
    wake();
    if (!app.fullscreen) setFullscreen(true);
  });
  onDestroy(() => {
    clearTimeout(advanceTimer);
    clearTimeout(idleTimer);
    // Leave the screen as it was, unless the viewer is taking over again.
    if (!wasFullscreen && app.fullscreen) setFullscreen(false);
  });
</script>

<svelte:window onkeydowncapture={onKey} onmousemove={wake} />

<div class="show" class:idle style:--bg={bg} role="dialog" aria-label="Slideshow" aria-modal="true">
  {#each layers as l (l.key)}
    <div class="layer" class:ready={l.ready}>
      {#if l.offline}
        {#if l.p.thumb}
          <img src={previewUrl(l.p)} alt={l.p.filename} draggable="false" onload={() => loaded(l)} onerror={() => loaded(l)} />
        {:else}
          <div class="missing"><CloudOff size={28} /> {l.p.filename}</div>
        {/if}
      {:else if isVideo(l.p)}
        <!-- Home videos have no caption files; the empty track marks that. -->
        <video
          src={photoUrl(l.p)}
          poster={l.p.thumb ? thumbUrl(l.p) : undefined}
          autoplay
          playsinline
          onloadeddata={() => loaded(l)}
          onerror={() => { loaded(l); setTimeout(() => current?.key === l.key && go(1), 1500); }}
          onended={() => { if (playing && current?.key === l.key) go(1); }}
        ><track kind="captions" /></video>
      {:else}
        <img
          src={photoUrl(l.p)}
          alt={l.p.filename}
          draggable="false"
          onload={() => loaded(l)}
          onerror={(e) => {
            // Couldn't decode the file: fall back to the thumbnail if there is one.
            const img = e.currentTarget as HTMLImageElement;
            if (l.p.thumb && !img.src.includes("thumb")) img.src = previewUrl(l.p);
            else loaded(l);
          }}
        />
      {/if}
    </div>
  {/each}

  {#if current && showCaption}
    <div class="caption">
      <strong>{current.p.filename}</strong>
      {#if current.p.taken_at}<span>{formatDate(current.p.taken_at)}</span>{/if}
      {#if current.offline}<span class="off"><CloudOff size={12} /> Drive not connected</span>{/if}
    </div>
  {/if}

  <div class="controls" role="toolbar" aria-label="Slideshow controls" tabindex="-1">
    <span class="count">{pos + 1} / {photos.length}</span>
    <button class="ghost icon" onclick={() => { go(-1); wake(); }} title="Previous (←)" aria-label="Previous"><ChevronLeft size={20} /></button>
    <button class="ghost icon play" onclick={togglePlay} title={playing ? "Pause (Space)" : "Play (Space)"} aria-label={playing ? "Pause" : "Play"}>
      {#if playing}<Pause size={20} />{:else}<Play size={20} />{/if}
    </button>
    <button class="ghost icon" onclick={() => { go(1); wake(); }} title="Next (→)" aria-label="Next"><ChevronRight size={20} /></button>
    <select value={seconds} onchange={(e) => setSeconds(Number((e.target as HTMLSelectElement).value))} title="Time per photo (− / +)">
      {#each SPEEDS as s}<option value={s}>{s} s</option>{/each}
    </select>
    <button class="ghost icon" class:on={shuffle} onclick={toggleShuffle} title="Shuffle (S)" aria-label="Shuffle" aria-pressed={shuffle}><Shuffle size={17} /></button>
    <button class="ghost icon" onclick={() => toggleFullscreen()} title={app.fullscreen ? "Leave full screen (F)" : "Full screen (F)"} aria-label="Full screen">
      {#if app.fullscreen}<Shrink size={17} />{:else}<Expand size={17} />{/if}
    </button>
    <button class="ghost icon" onclick={close} title="End slideshow (Esc)" aria-label="End slideshow"><X size={18} /></button>
  </div>
</div>

<style>
  .show { position: fixed; inset: 0; z-index: 700; background: var(--bg); overflow: hidden; color: #eee; }
  .show.idle { cursor: none; }
  .layer { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; opacity: 0; transition: opacity .8s ease; }
  .layer.ready { opacity: 1; }
  .layer img, .layer video { max-width: 100%; max-height: 100%; object-fit: contain; }
  .missing { display: flex; flex-direction: column; align-items: center; gap: 10px; color: #999; }
  .caption {
    position: absolute; left: 18px; bottom: 18px; display: flex; flex-direction: column; gap: 2px;
    padding: 6px 10px; border-radius: 8px; background: rgba(0,0,0,.45); font-size: 12px; transition: opacity .4s;
  }
  .caption span { color: #ccc; }
  .caption .off { display: flex; align-items: center; gap: 4px; }
  .controls {
    position: absolute; left: 50%; bottom: 18px; transform: translateX(-50%);
    display: flex; align-items: center; gap: 6px; padding: 6px 10px; border-radius: 12px;
    background: rgba(18, 19, 22, .82); backdrop-filter: blur(8px); transition: opacity .3s;
  }
  .idle .controls, .idle .caption { opacity: 0; pointer-events: none; }
  .controls button { color: #eee; }
  .controls button:hover { background: rgba(255,255,255,.12) !important; }
  .controls .on { color: var(--accent); }
  .controls select { width: auto; height: 28px; padding: 2px 6px; background: rgba(255,255,255,.08); color: #eee; border-color: rgba(255,255,255,.15); }
  .count { font-size: 12px; color: #bbb; font-variant-numeric: tabular-nums; min-width: 56px; text-align: center; }
</style>
