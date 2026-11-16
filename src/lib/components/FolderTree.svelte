<script lang="ts">
  import FolderTree from "./FolderTree.svelte";
  import { ChevronRight, Folder, FolderOpen } from "@lucide/svelte";
  import { api, type FolderNode } from "../api";
  import { app, setScope, dropOnFolder } from "../store.svelte";

  let { path, depth = 1, rootId, oncontext }: { path: string; depth?: number; rootId: number; oncontext: (e: MouseEvent, node: FolderNode) => void } = $props();

  let children = $state<FolderNode[]>([]);
  let expanded = $state<Record<string, boolean>>({});
  let dropTarget = $state<string | null>(null);

  $effect(() => {
    app.folderVersion;
    api.folderChildren(path).then((c) => (children = c)).catch(() => (children = []));
  });

  function select(n: FolderNode) {
    setScope({ kind: "folder", path: n.path, name: n.name, rootId });
  }
</script>

{#each children as n (n.path)}
  {@const active = app.scope.kind === "folder" && app.scope.path === n.path}
  <div
    class="row"
    class:active
    class:offline={!n.online}
    class:drop={dropTarget === n.path}
    style:padding-left={`${8 + depth * 14}px`}
    role="treeitem"
    aria-selected={active}
    aria-expanded={n.has_children ? !!expanded[n.path] : undefined}
    tabindex="-1"
    draggable={n.online}
    ondragstart={(e) => { e.dataTransfer?.setData("application/x-lumio-folder", n.path); }}
    ondragover={(e) => { if (n.online) { e.preventDefault(); dropTarget = n.path; if (e.dataTransfer) e.dataTransfer.dropEffect = e.altKey ? "copy" : "move"; } }}
    ondragleave={() => (dropTarget = null)}
    ondrop={(e) => { e.preventDefault(); dropTarget = null; dropOnFolder(n.path, e); }}
    onclick={() => select(n)}
    ondblclick={() => (expanded[n.path] = !expanded[n.path])}
    oncontextmenu={(e) => { e.preventDefault(); oncontext(e, n); }}
    onkeydown={() => {}}
  >
    <button class="twisty" class:open={expanded[n.path]} style:visibility={n.has_children ? "visible" : "hidden"} onclick={(e) => { e.stopPropagation(); expanded[n.path] = !expanded[n.path]; }} aria-label="Expand">
      <ChevronRight size={12} />
    </button>
    {#if expanded[n.path]}<FolderOpen size={14} />{:else}<Folder size={14} />{/if}
    <span class="name">{n.name}</span>
    {#if n.count}<span class="count">{n.count.toLocaleString()}</span>{/if}
  </div>
  {#if expanded[n.path] && n.has_children}
    <FolderTree path={n.path} depth={depth + 1} {rootId} {oncontext} />
  {/if}
{/each}

<style>
  .row {
    display: flex; align-items: center; gap: 5px; height: 26px; padding-right: 10px;
    border-radius: 6px; margin: 0 6px; color: var(--text-2); white-space: nowrap;
  }
  .row:hover { background: var(--bg-3); color: var(--text); }
  .row.active { background: var(--accent-soft); color: var(--text); }
  .row.drop { outline: 2px solid var(--accent); outline-offset: -2px; background: var(--accent-soft); }
  .row.offline { opacity: 0.6; }
  .name { overflow: hidden; text-overflow: ellipsis; flex: 1; }
  .count { font-size: 11px; color: var(--text-3); }
  .twisty { background: none; border: none; padding: 0; width: 14px; height: 14px; color: var(--text-3); justify-content: center; }
  .twisty:hover { background: none !important; color: var(--text); }
  .twisty :global(svg) { transition: transform 0.12s; }
  .twisty.open :global(svg) { transform: rotate(90deg); }
</style>
