<script lang="ts">
  import { app } from "../store.svelte";

  // The photographer's name from Settings > General. Blank means the second line is hidden.
  const owner = $derived((app.settings.owner_name ?? "").trim());
</script>

<div class="plate" class:named={!!owner} data-tauri-drag-region title={owner ? `Frameloft by Light and Alley · ${owner}` : "Frameloft by Light and Alley"}>
  <img src="/frameloft-icon.png" alt="" draggable="false" data-tauri-drag-region />
  <div class="text" data-tauri-drag-region>
    <div class="brand" data-tauri-drag-region><strong>Frameloft</strong> by Light and Alley</div>
    {#if owner}<div class="owner" data-tauri-drag-region>{owner}</div>{/if}
  </div>
</div>

<style>
  /* Sized to sit above the sidebar, so the page title lines up with the photos. */
  .plate { width: 224px; flex-shrink: 0; display: flex; align-items: center; gap: 8px; min-width: 0; user-select: none; -webkit-user-select: none; }
  img { width: 34px; height: 34px; flex-shrink: 0; margin-left: -3px; }
  .text { min-width: 0; display: flex; flex-direction: column; line-height: 1.2; }
  .brand { font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .brand strong { color: var(--text-2); font-weight: 600; }
  .named .brand { font-size: 10.5px; letter-spacing: .02em; }
  /* On a narrow window keep just the logo so the toolbar buttons still fit. */
  @media (max-width: 1150px) { .plate { width: auto; } .text { display: none; } }
  @media (max-width: 1000px) { .plate { display: none; } }
  .owner { font-size: 14px; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
