<script lang="ts">
  import FolderTree from "./FolderTree.svelte";
  import { Images, Clock, Star, MapPin, MapPinOff, Sparkles, Plus, HardDrive, CloudOff, ChevronRight } from "@lucide/svelte";
  import { app, setScope, addFolderToLibrary, dropOnFolder, type Scope } from "../store.svelte";
  import { showMenu } from "../menu.svelte";
  import { folderMenu } from "../actions";
  import type { FolderNode, Root } from "../api";

  let expanded = $state<Record<number, boolean>>({});
  let dropRoot = $state<number | null>(null);

  const smart: { kind: Scope["kind"]; label: string; icon: typeof Images }[] = [
    { kind: "all", label: "All photos", icon: Images },
    { kind: "recent", label: "Recently added", icon: Clock },
    { kind: "rated", label: "Rated", icon: Star },
    { kind: "geotagged", label: "On the map", icon: MapPin },
    { kind: "nogps", label: "Without location", icon: MapPinOff },
  ];

  function rootNode(r: Root): FolderNode {
    return { path: r.path, name: r.name, count: r.count, has_children: true, online: r.online };
  }
</script>

<aside class="sidebar">
  <div class="section-title">Library</div>
  {#each smart as s}
    <button class="item" class:active={app.scope.kind === s.kind} onclick={() => setScope({ kind: s.kind } as Scope)}>
      <s.icon size={15} /> <span>{s.label}</span>
    </button>
  {/each}
  {#if app.scope.kind === "ai"}
    <button class="item active"><Sparkles size={15} /> <span class="ell">{app.scope.title}</span></button>
  {/if}

  <div class="section-title">
    Folders
    <button class="ghost icon add" title="Add a folder to the library" onclick={() => addFolderToLibrary()}><Plus size={14} /></button>
  </div>
  <div class="tree" role="tree">
    {#each app.roots as r (r.id)}
      {@const active = app.scope.kind === "folder" && app.scope.path === r.path}
      <div
        class="root"
        class:active
        class:offline={!r.online}
        class:drop={dropRoot === r.id}
        role="treeitem"
        aria-selected={active}
        tabindex="-1"
        title={r.online ? r.path : `${r.path}\nDrive not connected: showing saved thumbnails`}
        onclick={() => setScope({ kind: "folder", path: r.path, name: r.name, rootId: r.id })}
        ondblclick={() => (expanded[r.id] = !expanded[r.id])}
        oncontextmenu={(e) => showMenu(e, folderMenu(rootNode(r), r))}
        ondragover={(e) => { if (r.online) { e.preventDefault(); dropRoot = r.id; } }}
        ondragleave={() => (dropRoot = null)}
        ondrop={(e) => { e.preventDefault(); dropRoot = null; dropOnFolder(r.path, e); }}
        onkeydown={() => {}}
      >
        <button class="twisty" class:open={expanded[r.id]} onclick={(e) => { e.stopPropagation(); expanded[r.id] = !expanded[r.id]; }} aria-label="Expand">
          <ChevronRight size={12} />
        </button>
        {#if r.online}<HardDrive size={14} />{:else}<CloudOff size={14} />{/if}
        <span class="name">{r.name}</span>
        <span class="count">{r.count.toLocaleString()}</span>
      </div>
      {#if expanded[r.id]}
        <FolderTree path={r.path} rootId={r.id} oncontext={(e, n) => showMenu(e, folderMenu(n))} />
      {/if}
    {/each}
    {#if !app.roots.length}
      <div class="empty">
        <p>Add the folders where your photos live: on this computer, an external drive or a network share.</p>
        <button class="primary" onclick={() => addFolderToLibrary()}><Plus size={14} /> Add folder</button>
      </div>
    {/if}
  </div>
</aside>

<style>
  .sidebar { width: 250px; flex-shrink: 0; background: var(--bg-2); border-right: 1px solid var(--line); overflow-y: auto; padding-bottom: 20px; }
  .item {
    display: flex; align-items: center; gap: 8px; width: calc(100% - 12px); margin: 0 6px; height: 28px;
    background: none; border: none; color: var(--text-2); border-radius: 6px; padding: 0 10px;
  }
  .item:hover { background: var(--bg-3) !important; color: var(--text); }
  .item.active { background: var(--accent-soft) !important; color: var(--text); }
  .item :global(svg) { color: var(--text-3); }
  .item.active :global(svg) { color: var(--accent); }
  .ell { overflow: hidden; text-overflow: ellipsis; }
  .add { padding: 2px; }
  .root {
    display: flex; align-items: center; gap: 5px; height: 28px; margin: 0 6px; padding: 0 10px 0 8px;
    border-radius: 6px; color: var(--text); white-space: nowrap; font-weight: 500;
  }
  .root:hover { background: var(--bg-3); }
  .root.active { background: var(--accent-soft); }
  .root.drop { outline: 2px solid var(--accent); outline-offset: -2px; }
  .root.offline { color: var(--text-3); }
  .name { flex: 1; overflow: hidden; text-overflow: ellipsis; }
  .count { font-size: 11px; color: var(--text-3); font-weight: 400; }
  .twisty { background: none; border: none; padding: 0; width: 14px; height: 14px; color: var(--text-3); justify-content: center; }
  .twisty:hover { background: none !important; color: var(--text); }
  .twisty :global(svg) { transition: transform 0.12s; }
  .twisty.open :global(svg) { transform: rotate(90deg); }
  .empty { padding: 8px 16px; color: var(--text-2); line-height: 1.45; }
</style>
