<script lang="ts">
  // Moves made while a drive was away. Each one is already shown in its new folder; the
  // file itself follows when both drives are connected. Lumio checks once a minute, and
  // whenever the window comes back to the front.
  import { X, TriangleAlert, Clock } from "@lucide/svelte";
  import { api, basename } from "../api";
  import { app, cancelPending, loadPending, toast } from "../store.svelte";

  function close() {
    app.showPending = false;
  }

  async function retry() {
    await api.retryPendingMoves().catch(() => {});
    toast("Trying the waiting moves now…");
    setTimeout(loadPending, 3000);
  }
</script>

{#if app.showPending}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
    <div class="win" role="dialog" aria-modal="true" aria-label="Waiting moves" tabindex="-1"
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); close(); } }}>
      <header>
        <h3>Moves waiting for a drive</h3>
        <button class="ghost icon" onclick={close} aria-label="Close"><X size={16} /></button>
      </header>
      <p class="small">
        These photos already show in their new folder. The files move as soon as both drives are connected. Nothing is
        overwritten: if a file with the same name is already there, the photo gets a name like “IMG_0001 (2).jpg”.
      </p>
      <div class="list">
        {#each app.pending as m (m.photo_id)}
          <div class="item" class:bad={!!m.error}>
            <span class="icon">{#if m.error}<TriangleAlert size={14} />{:else}<Clock size={14} />{/if}</span>
            <div class="what">
              <div class="name">{m.filename}</div>
              <div class="path" title={`${m.from} → ${m.to}`}>{basename(m.from)} → {basename(m.to)}</div>
              {#if m.error}<div class="why">{m.error}</div>{/if}
            </div>
            <button onclick={() => cancelPending([m.photo_id])} title="Leave the file where it is and put the photo back in its old folder">Cancel move</button>
          </div>
        {:else}
          <p class="small">Nothing is waiting.</p>
        {/each}
      </div>
      <div class="buttons">
        {#if app.pending.length > 1}
          <button onclick={() => cancelPending(app.pending.map((m) => m.photo_id))}>Cancel all</button>
        {/if}
        <span class="spacer"></span>
        <button onclick={retry}>Try now</button>
        <button class="primary" onclick={close}>Done</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 1600; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .win { width: 560px; max-width: calc(100vw - 40px); max-height: calc(100vh - 40px); background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 16px 18px; box-shadow: var(--shadow); line-height: 1.45; display: flex; flex-direction: column; }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
  h3 { margin: 0; font-size: 15px; }
  p { margin: 4px 0 10px; color: var(--text-2); }
  .small { font-size: 12px; }
  .list { overflow-y: auto; min-height: 60px; max-height: 50vh; border: 1px solid var(--line-2); border-radius: 8px; background: var(--bg); padding: 4px 8px; }
  .item { display: flex; gap: 10px; align-items: center; padding: 6px 0; border-bottom: 1px solid var(--line); }
  .item:last-child { border-bottom: none; }
  .icon { color: var(--text-2); display: flex; }
  .item.bad .icon { color: var(--danger); }
  .what { flex: 1; min-width: 0; }
  .name, .path { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .name { font-size: 12.5px; }
  .path { font-size: 11.5px; color: var(--text-2); }
  .why { font-size: 11.5px; color: var(--danger); }
  .buttons { display: flex; gap: 8px; margin-top: 12px; }
  .spacer { flex: 1; }
</style>
