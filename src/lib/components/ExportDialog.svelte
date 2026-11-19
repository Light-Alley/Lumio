<script lang="ts">
  // Save resized JPEG copies of the selected photos for a website (Squarespace and similar).
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { X, Check, FolderOpen } from "@lucide/svelte";
  import { api, errText } from "../api";
  import { publish, webExport, remember, keep, type WebExportReport, type WebExportProgress } from "../publish.svelte";

  const SIZES: [number | null, string][] = [
    [1500, "1500 px (small, fastest pages)"],
    [2048, "2048 px"],
    [2500, "2500 px (Squarespace's recommended size)"],
    [3000, "3000 px"],
    [null, "Full size"],
  ];

  let longEdge = $state<number | null>(remember("export.longEdge", 2500));
  let quality = $state<number>(remember("export.quality", 85));
  let stripLocation = $state<boolean>(remember("export.stripLocation", true));
  let step = $state<"options" | "running" | "done">("options");
  let progress = $state<WebExportProgress | null>(null);
  let report = $state<WebExportReport | null>(null);
  let error = $state("");

  const ids = $derived(publish.exportIds ?? []);
  const count = $derived(ids.length);

  let opened = false;
  $effect(() => {
    if (publish.exportIds && !opened) {
      opened = true;
      step = "options";
      report = null;
      error = "";
    } else if (!publish.exportIds) opened = false;
  });

  async function run() {
    const last = remember<string | undefined>("export.folder", undefined);
    const folder = await openDialog({ directory: true, multiple: false, title: "Choose a folder for the web copies", defaultPath: last });
    if (typeof folder !== "string") return;
    keep("export.folder", folder);
    keep("export.longEdge", longEdge);
    keep("export.quality", quality);
    keep("export.stripLocation", stripLocation);
    step = "running";
    error = "";
    progress = { done: 0, total: count, current: "" };
    const un = await listen<WebExportProgress>("web-export-progress", (e) => (progress = e.payload));
    try {
      report = await webExport.run(ids, folder, { long_edge: longEdge, quality, strip_location: stripLocation });
      step = "done";
    } catch (e) {
      error = errText(e);
      step = "options";
    } finally {
      un();
    }
  }

  function close() {
    if (step === "running") return;
    publish.exportIds = null;
  }
</script>

{#if publish.exportIds}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
    <div class="win" role="dialog" aria-modal="true" aria-label="Export web-ready copies" tabindex="-1" onkeydown={(e) => e.key === "Escape" && (e.stopPropagation(), close())}>
      <header>
        <h3>Export web-ready copies of {count === 1 ? "1 photo" : `${count} photos`}</h3>
        {#if step !== "running"}<button class="ghost icon" onclick={close} aria-label="Close"><X size={16} /></button>{/if}
      </header>

      {#if step === "options"}
        <p class="muted">Makes upright JPEG copies sized for a website such as Squarespace, ready to drag into an image block or gallery. Your originals are not changed.</p>
        <label class="field"><span>Longest side</span>
          <select bind:value={longEdge}>
            {#each SIZES as [v, label]}<option value={v}>{label}</option>{/each}
          </select>
        </label>
        <label class="field"><span>JPEG quality: {quality} {quality >= 90 ? "(larger files)" : quality <= 70 ? "(smaller files, may show artefacts)" : ""}</span>
          <input type="range" min="60" max="95" step="1" bind:value={quality} />
        </label>
        <label class="check"><input type="checkbox" bind:checked={stripLocation} />
          <span>Remove the location (GPS) from the copies. Recommended for photos taken at home. Captions, keywords and copyright are kept.</span></label>
        <p class="faint small">Squarespace suggests images 1500 to 2500 pixels wide and under 20 MB each. Videos are skipped.</p>
        {#if error}<p class="err">{error}</p>{/if}
        <div class="buttons">
          <button onclick={close}>Cancel</button>
          <button class="primary" onclick={run}><FolderOpen size={14} /> Choose folder and export</button>
        </div>

      {:else if step === "running" && progress}
        <p>Exporting {Math.min(progress.done + 1, Math.max(progress.total, 1))} of {progress.total}</p>
        <div class="bar"><div style:width={`${progress.total ? (progress.done / progress.total) * 100 : 0}%`}></div></div>
        <p class="faint small file">{progress.current}</p>
        <div class="buttons"><button onclick={() => webExport.cancel()}>Stop</button></div>

      {:else if step === "done" && report}
        {#if report.done}<p class="ok"><Check size={14} /> Saved {report.done === 1 ? "1 copy" : `${report.done} copies`}.</p>{/if}
        {#if report.cancelled}<p class="muted">Stopped before the end; the copies made so far are in the folder.</p>{/if}
        {#if report.skipped_videos}<p class="muted">{report.skipped_videos === 1 ? "1 video was" : `${report.skipped_videos} videos were`} skipped.</p>{/if}
        {#if report.failed.length}
          <p class="err">{report.failed.length === 1 ? "1 photo" : `${report.failed.length} photos`} could not be exported:</p>
          <ul class="failed">{#each report.failed as [name, why]}<li><strong>{name}</strong>: {why}</li>{/each}</ul>
        {/if}
        <div class="buttons">
          {#if report.done}<button onclick={() => api.reveal(undefined, report!.folder).catch(() => {})}><FolderOpen size={14} /> Show folder</button>{/if}
          <button class="primary" onclick={close}>Done</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 1600; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .win { width: 480px; max-width: calc(100vw - 40px); max-height: calc(100vh - 40px); overflow-y: auto; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 16px 18px; box-shadow: var(--shadow); line-height: 1.45; }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
  h3 { margin: 0; font-size: 15px; }
  p { margin: 6px 0 10px; }
  .small { font-size: 11.5px; }
  .check { display: flex; gap: 8px; align-items: flex-start; margin-bottom: 8px; }
  .check input { margin-top: 3px; accent-color: var(--accent); }
  input[type="range"] { width: 100%; accent-color: var(--accent); }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }
  .bar { height: 8px; border-radius: 4px; background: var(--bg-4); overflow: hidden; margin: 10px 0 4px; }
  .bar div { height: 100%; background: var(--accent); transition: width .25s; }
  .file { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ok { color: var(--success); display: flex; gap: 6px; align-items: center; }
  .err { color: var(--danger); white-space: pre-wrap; }
  .failed { max-height: 140px; overflow-y: auto; padding-left: 18px; font-size: 12px; user-select: text; -webkit-user-select: text; }
</style>
