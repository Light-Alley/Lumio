<script lang="ts">
  import { Loader } from "@lucide/svelte";
  import { app, selection } from "../store.svelte";
</script>

<footer class="status">
  <span>{app.photos.length.toLocaleString()} photo{app.photos.length === 1 ? "" : "s"}{#if selection.size} · {selection.size.toLocaleString()} selected{/if}</span>
  <span class="spacer"></span>
  {#if app.op.total && app.op.done < app.op.total}
    <span class="job"><Loader size={12} class="spin" /> {app.op.message} ({app.op.done + 1}/{app.op.total})</span>
  {/if}
  {#if app.progress.active}
    <span class="job">
      <Loader size={12} class="spin" /> {app.progress.message}
      {#if app.progress.total}<span class="bar"><span style:width={`${(app.progress.done / app.progress.total) * 100}%`}></span></span>{/if}
    </span>
  {/if}
</footer>

<style>
  .status { height: 26px; flex-shrink: 0; display: flex; align-items: center; gap: 14px; padding: 0 12px; background: var(--bg-2); border-top: 1px solid var(--line); color: var(--text-2); font-size: 11.5px; }
  .spacer { flex: 1; }
  .job { display: flex; align-items: center; gap: 6px; max-width: 50%; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .bar { width: 120px; height: 4px; background: var(--bg-4); border-radius: 2px; overflow: hidden; display: inline-block; }
  .bar span { display: block; height: 100%; background: var(--accent); }
  :global(.spin) { animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
