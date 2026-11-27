<script lang="ts">
  import { ImageOff, FolderOpen, Plus } from "@lucide/svelte";
  import { app, addFolderToLibrary, saveSetting, loadPhotos } from "../store.svelte";

  const folderEmptyButHasSubs = $derived(app.scope.kind === "folder" && !app.recursive && app.photos.length === 0 && !app.text && !app.minRating && !app.label);
</script>

{#if !app.loading && app.photos.length === 0}
  <div class="empty">
    {#if !app.roots.length}
      <img class="brand" src="/logo.png" alt="Light & Alley Photography" draggable="false" />
      <h2>Welcome to Frameloft</h2>
      <p>Add a folder of photos to get started. It can be on this computer, an external drive or a network drive. Frameloft keeps small thumbnails so you can still browse them when the drive is unplugged.</p>
      <button class="primary" onclick={() => addFolderToLibrary()}><Plus size={14} /> Add a folder</button>
    {:else if folderEmptyButHasSubs}
      <FolderOpen size={40} strokeWidth={1.2} />
      <p>No photos directly in this folder.</p>
      <button onclick={() => { app.recursive = true; saveSetting("recursive", "1"); loadPhotos(); }}>Show photos from subfolders</button>
    {:else}
      <ImageOff size={40} strokeWidth={1.2} />
      <p>No photos match.</p>
    {/if}
  </div>
{/if}

<style>
  .empty {
    position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 10px; color: var(--text-2); text-align: center; padding: 40px; pointer-events: none;
  }
  .empty > * { pointer-events: auto; }
  .brand { width: 170px; height: auto; margin-bottom: 8px; opacity: .9; }
  :global(:root[data-theme="light"]) .brand { filter: invert(0.7); }
  .empty h2 { margin: 6px 0 0; color: var(--text); font-weight: 600; }
  .empty p { max-width: 440px; line-height: 1.5; margin: 0 0 6px; }
</style>
