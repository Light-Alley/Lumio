<script lang="ts">
  // Red, green and blue levels drawn over each other, with brightness (luminance) as a
  // line on top. Shadows are on the left, highlights on the right. Counted by the app from
  // the preview it keeps, so it also works when the drive is unplugged.
  import { api, errText, type Histogram } from "../api";

  let { id, version }: { id: number; version: string } = $props();

  let hist = $state<Histogram | null>(null);
  let error = $state("");
  let seq = 0;

  $effect(() => {
    version; // count again when the photo or its preview changes
    const mine = ++seq;
    error = "";
    api.histogram(id).then(
      (h) => { if (mine === seq) hist = h; },
      (e) => { if (mine === seq) { hist = null; error = errText(e); } },
    );
  });

  const W = 256, H = 100;

  // Scale to the tallest level, ignoring pure black and pure white so a clipped sky or
  // shadow doesn't flatten the rest of the graph (those show as warnings below instead).
  const peak = $derived.by(() => {
    if (!hist) return 1;
    let m = 1;
    for (const ch of [hist.r, hist.g, hist.b, hist.l]) for (let i = 1; i < 255; i++) if (ch[i] > m) m = ch[i];
    return m;
  });

  function area(ch: number[]): string {
    let d = `M0 ${H}`;
    for (let i = 0; i < 256; i++) d += ` L${i} ${(H - Math.min(1, ch[i] / peak) * H).toFixed(1)}`;
    return d + ` L${W - 1} ${H} Z`;
  }
  function line(ch: number[]): string {
    let d = "";
    for (let i = 0; i < 256; i++) d += `${i ? " L" : "M"}${i} ${(H - Math.min(1, ch[i] / peak) * H).toFixed(1)}`;
    return d;
  }
  const pct = (x: number) => (x < 0.001 ? "" : x < 0.01 ? "<1%" : `${Math.round(x * 100)}%`);
</script>

<div class="hist">
  {#if hist}
    <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label="Histogram">
      <line x1="64" x2="64" y1="0" y2={H} class="grid" />
      <line x1="128" x2="128" y1="0" y2={H} class="grid" />
      <line x1="192" x2="192" y1="0" y2={H} class="grid" />
      <g class="rgb">
        <path d={area(hist.r)} fill="#ff4d4d" />
        <path d={area(hist.g)} fill="#4ddb4d" />
        <path d={area(hist.b)} fill="#4d7dff" />
      </g>
      <path d={line(hist.l)} class="luma" vector-effect="non-scaling-stroke" />
    </svg>
    <div class="scale small faint">
      <span title="Pixels that are pure black">{#if pct(hist.shadows_clipped)}<b class="clip">◀ {pct(hist.shadows_clipped)} black</b>{:else}Shadows{/if}</span>
      <span>Midtones</span>
      <span title="Pixels with a channel at pure white">{#if pct(hist.highlights_clipped)}<b class="clip">{pct(hist.highlights_clipped)} clipped ▶</b>{:else}Highlights{/if}</span>
    </div>
  {:else if error}
    <div class="msg small muted">{error === "No preview yet" ? "The histogram appears once the thumbnail is ready." : error}</div>
  {:else}
    <div class="msg small faint">Counting…</div>
  {/if}
</div>

<style>
  .hist { width: 100%; height: 100%; display: flex; flex-direction: column; justify-content: center; }
  svg { width: 100%; height: 130px; background: var(--bg); border: 1px solid var(--line); border-radius: 4px; display: block; }
  .grid { stroke: var(--line); stroke-width: 1; vector-effect: non-scaling-stroke; }
  .rgb path { opacity: 0.55; mix-blend-mode: screen; }
  :global([data-theme="light"]) .rgb path { mix-blend-mode: multiply; opacity: 0.45; }
  .luma { fill: none; stroke: var(--text); stroke-width: 1.25; opacity: 0.85; }
  .scale { display: flex; justify-content: space-between; margin-top: 5px; font-size: 11px; }
  .clip { color: var(--star); font-weight: 600; }
  .msg { text-align: center; padding: 0 10px; }
  .small { font-size: 11.5px; }
</style>
