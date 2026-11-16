<script lang="ts">
  import { tick } from "svelte";
  import { app } from "../store.svelte";
  let value = $state("");
  let inputEl: HTMLInputElement | undefined = $state();
  let okEl: HTMLButtonElement | undefined = $state();

  $effect(() => {
    const d = app.dialog;
    if (!d) return;
    value = d.value ?? "";
    tick().then(() => {
      if (inputEl) {
        inputEl.focus();
        const dot = value.lastIndexOf(".");
        inputEl.setSelectionRange(0, dot > 0 ? dot : value.length);
      } else okEl?.focus();
    });
  });

  function done(ok: boolean) {
    const d = app.dialog;
    if (!d) return;
    app.dialog = null;
    if (d.kind === "prompt") d.resolve(ok ? value.trim() || null : null);
    else d.resolve(ok);
  }
</script>

{#if app.dialog}
  {@const d = app.dialog}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && done(false)}>
    <div class="dialog" role="dialog" aria-modal="true" aria-label={d.title} tabindex="-1"
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); done(false); } if (e.key === "Enter" && d.kind === "prompt") { e.preventDefault(); done(true); } }}>
      <h3>{d.title}</h3>
      {#if d.message}<p>{d.message}</p>{/if}
      {#if d.kind === "prompt"}<input type="text" bind:this={inputEl} bind:value />{/if}
      <div class="buttons">
        <button onclick={() => done(false)}>Cancel</button>
        <button bind:this={okEl} class={d.danger ? "danger" : "primary"} onclick={() => done(true)}>{d.okLabel ?? "OK"}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 2000; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .dialog { width: 420px; max-width: calc(100vw - 40px); background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 18px; box-shadow: var(--shadow); }
  h3 { margin: 0 0 8px; font-size: 15px; }
  p { margin: 0 0 12px; color: var(--text-2); line-height: 1.5; white-space: pre-wrap; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
</style>
