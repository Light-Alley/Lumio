<script lang="ts">
  import { Download, RefreshCw } from "@lucide/svelte";
  import { updates, installUpdate, openReleasePage, skipThisVersion } from "../updates.svelte";
  import { formatBytes } from "../api";

  let box: HTMLDivElement | undefined = $state();
  $effect(() => { if (updates.open) box?.focus(); });

  function close() {
    if (!updates.installing) updates.open = false;
  }
  /** Release notes are Markdown; show them as plain text without the # and ** marks. */
  function plain(md: string): string {
    return md
      .replace(/\r/g, "")
      .replace(/^#{1,6}\s*/gm, "")
      .replace(/^\s*[-*]\s+/gm, "• ")
      .replace(/\*\*(.+?)\*\*/g, "$1")
      .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
      .replace(/`([^`]+)`/g, "$1")
      .replace(/\n{3,}/g, "\n\n")
      .trim();
  }
  const notes = $derived(updates.info ? plain(updates.info.notes) : "");
  const pct = $derived(updates.total ? Math.min(100, Math.round((updates.downloaded / updates.total) * 100)) : null);
</script>

{#if updates.open && updates.info}
  {@const u = updates.info}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
    <div class="dialog" bind:this={box} role="dialog" aria-modal="true" aria-label="Update available" tabindex="-1"
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); close(); } }}>
      <h3>Lumio {u.latest} is available</h3>
      <p class="muted">You have version {u.current}.{#if u.published}{" "}Released {new Date(u.published).toLocaleDateString()}.{/if}</p>
      {#if notes}
        <div class="notes">{notes}</div>
      {/if}
      {#if updates.installing}
        <div class="progress" aria-live="polite">
          <div class="bar"><div class="fill" style:width={pct === null ? "35%" : `${pct}%`} class:indeterminate={pct === null}></div></div>
          <span class="muted">Downloading {formatBytes(updates.downloaded)}{#if updates.total} of {formatBytes(updates.total)}{/if}. Lumio restarts by itself when it's installed.</span>
        </div>
      {:else if !u.can_install}
        <p class="muted small">Download the installer for your computer from the release page and install it over this copy, as you did the first time. Your library, settings and photos stay as they are.</p>
      {/if}
      <div class="buttons">
        <button class="ghost left" disabled={updates.installing} onclick={skipThisVersion}>Skip This Version</button>
        <button disabled={updates.installing} onclick={close}>Later</button>
        {#if u.can_install}
          <button class="primary" disabled={updates.installing} onclick={installUpdate}><RefreshCw size={14} /> Install and Restart</button>
        {:else}
          <button class="primary" onclick={() => { openReleasePage(); close(); }}><Download size={14} /> Download</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 2100; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .dialog { width: 520px; max-width: calc(100vw - 40px); max-height: calc(100vh - 60px); display: flex; flex-direction: column; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 18px; box-shadow: var(--shadow); }
  h3 { margin: 0 0 4px; font-size: 15px; }
  p { margin: 0 0 12px; line-height: 1.5; }
  .small { font-size: 12px; }
  .notes { flex: 1; min-height: 60px; max-height: 320px; overflow-y: auto; white-space: pre-wrap; line-height: 1.5; font-size: 12.5px; background: var(--bg); border: 1px solid var(--line); border-radius: 8px; padding: 10px 12px; margin-bottom: 12px; user-select: text; -webkit-user-select: text; }
  .progress { display: flex; flex-direction: column; gap: 6px; margin-bottom: 8px; font-size: 12px; }
  .bar { height: 6px; background: var(--bg-3); border-radius: 3px; overflow: hidden; }
  .fill { height: 100%; background: var(--accent); transition: width .2s; }
  .fill.indeterminate { animation: slide 1.2s ease-in-out infinite; }
  @keyframes slide { from { transform: translateX(-100%); } to { transform: translateX(300%); } }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
  .buttons .left { margin-right: auto; }
</style>
