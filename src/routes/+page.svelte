<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import Toolbar from "$lib/components/Toolbar.svelte";
  import Grid from "$lib/components/Grid.svelte";
  import ListView from "$lib/components/ListView.svelte";
  import Filmstrip from "$lib/components/Filmstrip.svelte";
  import TabBar from "$lib/components/TabBar.svelte";
  import MapView from "$lib/components/MapView.svelte";
  import Viewer from "$lib/components/Viewer.svelte";
  import Inspector from "$lib/components/Inspector.svelte";
  import ChatPanel from "$lib/components/ChatPanel.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import Dialog from "$lib/components/Dialog.svelte";
  import Toasts from "$lib/components/Toasts.svelte";
  import StatusBar from "$lib/components/StatusBar.svelte";
  import Settings from "$lib/components/Settings.svelte";
  import SidebarResizer from "$lib/components/SidebarResizer.svelte";
  import { ChevronLeft, ChevronRight } from "@lucide/svelte";
  import { app, init, setPanel } from "$lib/store.svelte";
  import { watchVideoThumbs } from "$lib/videoThumbs.svelte";
  import SmugMugDialog from "$lib/components/SmugMugDialog.svelte";
  import PendingMoves from "$lib/components/PendingMoves.svelte";
  import HistoryPanel from "$lib/components/HistoryPanel.svelte";
  import ExportDialog from "$lib/components/ExportDialog.svelte";
  import RenameDialog from "$lib/components/RenameDialog.svelte";
  import CollectionDialog from "$lib/components/CollectionDialog.svelte";
  import DuplicatesDialog from "$lib/components/DuplicatesDialog.svelte";
  import Slideshow from "$lib/components/Slideshow.svelte";
  import UpdateDialog from "$lib/components/UpdateDialog.svelte";
  import ShortcutsDialog from "$lib/components/ShortcutsDialog.svelte";
  import { watchFullscreen } from "$lib/fullscreen";
  import { startMenubar } from "$lib/menubar.svelte";
  import { startUpdateChecks } from "$lib/updates.svelte";

  onMount(() => { init().then(startUpdateChecks); watchFullscreen(); startMenubar(); });
  watchVideoThumbs();
  $effect(() => { document.documentElement.dataset.theme = app.theme; });
</script>

<div class="shell" oncontextmenu={(e) => { if (!(e.target as HTMLElement).closest("input, textarea")) e.preventDefault(); }} role="application">
  <Toolbar />
  <div class="body">
    <Sidebar />
    <SidebarResizer />
    <div class="center">
      <TabBar />
      <main>
        {#if app.view === "grid"}<Grid />{:else if app.view === "list"}<ListView />{:else if app.view === "filmstrip"}<Filmstrip />{:else}<MapView />{/if}
      </main>
    </div>
    {#if app.panel}
      <aside class="panel">
        <button class="edge collapse" title="Hide this panel (I)" aria-label="Hide panel" onclick={() => setPanel(null)}><ChevronRight size={13} /></button>
        <div class="pane" class:hidden={app.panel !== "info"}><Inspector /></div>
        <div class="pane" class:hidden={app.panel !== "ai"}><ChatPanel /></div>
      </aside>
    {:else}
      <button class="edge expand" title="Show the info panel (I)" aria-label="Show info panel" onclick={() => setPanel("info")}><ChevronLeft size={13} /></button>
    {/if}
  </div>
  <StatusBar />
</div>

{#if app.viewerIndex >= 0}<Viewer />{/if}
{#if app.slideshow}<Slideshow />{/if}
<SmugMugDialog />
<PendingMoves />
<HistoryPanel />
<ExportDialog />
<RenameDialog />
<CollectionDialog />
<DuplicatesDialog />
<ContextMenu />
<Settings />
<UpdateDialog />
<ShortcutsDialog />
<Dialog />
<Toasts />

<style>
  .shell { height: 100vh; display: flex; flex-direction: column; }
  .body { flex: 1; display: flex; min-height: 0; }
  .center { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  main { flex: 1; min-height: 0; display: flex; min-width: 0; background: var(--bg); }
  .panel { width: 320px; flex-shrink: 0; border-left: 1px solid var(--line); background: var(--bg-2); position: relative; }
  .pane { position: absolute; inset: 0; }
  /* Small tab on the panel's left edge that hides it, and a slim strip that brings it back. */
  .edge {
    display: flex; align-items: center; justify-content: center; padding: 0; color: var(--text-3);
    background: var(--bg-2); border: 1px solid var(--line); border-radius: 0;
  }
  .edge:hover { color: var(--text); background: var(--bg-3) !important; }
  .collapse { position: absolute; z-index: 3; left: -12px; top: 50%; transform: translateY(-50%); width: 12px; height: 44px; border-right: none; border-radius: 6px 0 0 6px; }
  .expand { width: 14px; flex-shrink: 0; border-top: none; border-bottom: none; border-right: none; }
  .hidden { display: none; }
</style>
