<script lang="ts">
  import { X } from "@lucide/svelte";
  import { app, dismissToast } from "../store.svelte";
</script>

<div class="toasts">
  {#each app.toasts as t (t.id)}
    <div class="toast {t.kind}">
      <span class="text">{t.text}</span>
      {#if t.action}<button onclick={() => { t.action!.run(); dismissToast(t.id); }}>{t.action.label}</button>{/if}
      <button class="ghost icon" onclick={() => dismissToast(t.id)} aria-label="Dismiss"><X size={13} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts { position: fixed; bottom: 40px; right: 16px; z-index: 3000; display: flex; flex-direction: column; gap: 8px; align-items: flex-end; }
  .toast { display: flex; align-items: center; gap: 10px; max-width: 460px; background: var(--bg-2); border: 1px solid var(--line-2); border-left: 4px solid var(--accent); border-radius: 8px; padding: 9px 8px 9px 12px; box-shadow: var(--shadow); }
  .toast.error { border-left-color: var(--danger); }
  .toast.success { border-left-color: var(--success); }
  .text { white-space: pre-wrap; line-height: 1.4; flex: 1; user-select: text; -webkit-user-select: text; }
</style>
