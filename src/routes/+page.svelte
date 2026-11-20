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
  import { app, init } from "$lib/store.svelte";
  import { watchVideoThumbs } from "$lib/videoThumbs.svelte";
  import SmugMugDialog from "$lib/components/SmugMugDialog.svelte";
  import PendingMoves from "$lib/components/PendingMoves.svelte";
  import ExportDialog from "$lib/components/ExportDialog.svelte";
  import RenameDialog from "$lib/components/RenameDialog.svelte";

  onMount(() => { init(); });
  watchVideoThumbs();
  $effect(() => { document.documentElement.dataset.theme = app.theme; });
</script>

<div class="shell" oncontextmenu={(e) => { if (!(e.target as HTMLElement).closest("input, textarea")) e.preventDefault(); }} role="application">
  <Toolbar />
  <div class="body">
    <Sidebar />
    <div class="center">
      <TabBar />
      <main>
        {#if app.view === "grid"}<Grid />{:else if app.view === "list"}<ListView />{:else if app.view === "filmstrip"}<Filmstrip />{:else}<MapView />{/if}
      </main>
    </div>
    {#if app.panel}
      <aside class="panel">
        <div class:hidden={app.panel !== "info"}><Inspector /></div>
        <div class:hidden={app.panel !== "ai"}><ChatPanel /></div>
      </aside>
    {/if}
  </div>
  <StatusBar />
</div>

{#if app.viewerIndex >= 0}<Viewer />{/if}
<SmugMugDialog />
<PendingMoves />
<ExportDialog />
<RenameDialog />
<ContextMenu />
<Settings />
<Dialog />
<Toasts />

<style>
  .shell { height: 100vh; display: flex; flex-direction: column; }
  .body { flex: 1; display: flex; min-height: 0; }
  .center { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  main { flex: 1; min-height: 0; display: flex; min-width: 0; background: var(--bg); }
  .panel { width: 320px; flex-shrink: 0; border-left: 1px solid var(--line); background: var(--bg-2); position: relative; }
  .panel > div { position: absolute; inset: 0; }
  .hidden { display: none; }
</style>
