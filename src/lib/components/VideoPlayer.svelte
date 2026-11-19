<script lang="ts">
  // A small video player: play/pause, 10 s back and forward, a seek bar and volume.
  // It uses the system's own video support through the webview (nothing is bundled),
  // so which formats play depends on the computer; see docs/video.md.
  import { Play, Pause, Rewind, FastForward, Volume2, VolumeX, ExternalLink } from "@lucide/svelte";
  import { api, photoUrl, thumbUrl, formatDuration, isLinux, type PhotoLite } from "../api";

  let { p, autoplay = false, compact = false }: { p: PhotoLite; autoplay?: boolean; compact?: boolean } = $props();

  const SKIP = 10;
  let video: HTMLVideoElement | undefined = $state();
  let paused = $state(true);
  let time = $state(0);
  let duration = $state(0);
  let muted = $state(false);
  let volume = $state(readVolume());
  let unsupported = $state(false);
  let ready = $state(false);

  // Start over only when the file itself changes (the list is refreshed while indexing).
  const src = $derived(photoUrl(p));
  $effect(() => {
    src;
    unsupported = false;
    ready = false;
    time = 0;
    duration = p.duration ?? 0;
    paused = true;
  });

  function readVolume(): number {
    try {
      const v = Number(localStorage.getItem("lumio.volume"));
      return isFinite(v) && v > 0 && v <= 1 ? v : 1;
    } catch { return 1; }
  }
  function keepVolume() {
    try { localStorage.setItem("lumio.volume", String(volume)); } catch { /* not important */ }
  }

  export function toggle() {
    if (!video || unsupported) return;
    if (video.paused) video.play().catch(() => {});
    else video.pause();
  }
  export function skip(seconds: number) {
    if (!video || unsupported) return;
    const end = isFinite(video.duration) ? video.duration : Infinity;
    video.currentTime = Math.max(0, Math.min(end, video.currentTime + seconds));
  }
  export function toggleMute() { muted = !muted; }
  export function pause() { video?.pause(); }

  function seek(e: Event) {
    if (!video) return;
    video.currentTime = Number((e.target as HTMLInputElement).value);
  }
  function setVolume(e: Event) {
    volume = Number((e.target as HTMLInputElement).value);
    muted = volume === 0;
    keepVolume();
  }

  function onMeta() {
    if (!video) return;
    if (isFinite(video.duration)) duration = video.duration;
    // Some engines open a file whose picture they can't decode (e.g. HEVC on Windows
    // without the HEVC extension) and would play only the sound.
    if (video.videoWidth === 0 && video.videoHeight === 0) { unsupported = true; video.pause(); return; }
    ready = true;
  }

  const ext = $derived(p.ext.toUpperCase());
</script>

<div class="player" class:compact>
  {#if unsupported}
    {#if p.thumb}<img class="poster" src={thumbUrl(p)} alt="" draggable="false" />{/if}
    <div class="nope">
      <strong>Lumio can't play this {ext} video on this computer.</strong>
      <span>Open it in your usual video player instead.</span>
      {#if isLinux}<span class="hint">On Linux, installing the GStreamer plugins (gst-plugins-good, gst-plugins-bad and gst-libav) lets Lumio play many more formats.</span>{/if}
      <button onclick={() => api.openDefault([p.id])}><ExternalLink size={14} /> Open in video player</button>
    </div>
  {:else}
    {#key src}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={video}
        {src}
        poster={p.thumb ? thumbUrl(p) : undefined}
        preload="metadata"
        playsinline
        {autoplay}
        bind:paused
        bind:currentTime={time}
        bind:muted
        bind:volume
        onloadedmetadata={onMeta}
        ondurationchange={() => { if (video && isFinite(video.duration)) duration = video.duration; }}
        onerror={() => (unsupported = true)}
        onclick={toggle}
      ></video>
    {/key}
    <div class="bar" class:show={paused} role="toolbar" aria-label="Video controls" tabindex="-1" ondblclick={(e) => e.stopPropagation()}>
      <button class="ghost icon" onclick={() => skip(-SKIP)} disabled={!ready} title={`Back ${SKIP} seconds (J)`} aria-label={`Back ${SKIP} seconds`}><Rewind size={16} /></button>
      <button class="ghost icon play" onclick={toggle} disabled={!ready} title={paused ? "Play (K)" : "Pause (K)"} aria-label={paused ? "Play" : "Pause"}>
        {#if paused}<Play size={18} />{:else}<Pause size={18} />{/if}
      </button>
      <button class="ghost icon" onclick={() => skip(SKIP)} disabled={!ready} title={`Forward ${SKIP} seconds (L)`} aria-label={`Forward ${SKIP} seconds`}><FastForward size={16} /></button>
      <span class="time">{formatDuration(time)} / {formatDuration(duration)}</span>
      <input class="seek" type="range" min="0" max={duration || 0} step="0.1" value={time} oninput={seek} disabled={!ready} aria-label="Position" />
      <button class="ghost icon" onclick={toggleMute} title={muted ? "Unmute (M)" : "Mute (M)"} aria-label={muted ? "Unmute" : "Mute"}>
        {#if muted || volume === 0}<VolumeX size={16} />{:else}<Volume2 size={16} />{/if}
      </button>
      <input class="vol" type="range" min="0" max="1" step="0.05" value={muted ? 0 : volume} oninput={setVolume} aria-label="Volume" />
    </div>
  {/if}
</div>

<style>
  .player { position: absolute; inset: 20px; display: flex; align-items: center; justify-content: center; }
  .player.compact { inset: 12px; }
  video, .poster { max-width: 100%; max-height: 100%; object-fit: contain; background: #000; }
  .poster { opacity: .35; }
  .bar {
    position: absolute; left: 50%; bottom: 10px; transform: translateX(-50%); width: min(720px, calc(100% - 20px));
    display: flex; align-items: center; gap: 6px; padding: 6px 10px; border-radius: 10px;
    background: rgba(18, 19, 22, .82); backdrop-filter: blur(8px); color: #eee;
    opacity: 0; transition: opacity .15s;
  }
  .player:hover .bar, .bar.show, .bar:focus-within { opacity: 1; }
  .bar button { color: #eee; }
  .bar button:hover:not(:disabled) { background: rgba(255,255,255,.12) !important; }
  .time { font-size: 11.5px; font-variant-numeric: tabular-nums; white-space: nowrap; color: #ccc; }
  .seek { flex: 1; min-width: 60px; accent-color: var(--accent); }
  .vol { width: 80px; accent-color: var(--accent); }
  .compact .vol { width: 56px; }
  .nope {
    position: absolute; display: flex; flex-direction: column; align-items: center; gap: 8px; text-align: center;
    max-width: 440px; padding: 16px 20px; border-radius: 12px; background: rgba(0,0,0,.75); color: #eee; font-size: 12.5px;
  }
  .nope span { color: #bbb; }
  .nope .hint { font-size: 11.5px; }
  .nope button { display: flex; align-items: center; gap: 6px; margin-top: 4px; }
</style>
