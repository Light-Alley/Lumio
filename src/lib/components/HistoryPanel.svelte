<script lang="ts">
  // The undo history: every change Lumio made to files, newest first. Undoing an older
  // change first undoes everything after it, so each step finds the files where it left
  // them. ⌘Z / Ctrl+Z undoes the latest change from anywhere in the app.
  import { X, Undo2, Check, Trash2, TriangleAlert, FolderInput, FolderPlus, Copy, PencilLine, FolderPen } from "@lucide/svelte";
  import type { HistoryEntry } from "../api";
  import { app, confirm, latestUndoable, loadHistory, undo } from "../store.svelte";

  function close() {
    app.showHistory = false;
  }

  $effect(() => {
    if (app.showHistory) loadHistory();
  });

  const icons: Record<string, typeof Undo2> = {
    move: FolderInput, copy: Copy, rename: PencilLine, folder_rename: FolderPen, folder_move: FolderInput,
    folder_copy: Copy, folder_create: FolderPlus, trash: Trash2,
  };

  function when(ms: number): string {
    const d = new Date(ms);
    const today = new Date();
    const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    if (d.toDateString() === today.toDateString()) return time;
    const y = new Date(today);
    y.setDate(today.getDate() - 1);
    if (d.toDateString() === y.toDateString()) return `Yesterday ${time}`;
    return `${d.toLocaleDateString([], { day: "numeric", month: "short" })} ${time}`;
  }

  /** Changes that would be undone along with this one (newer and not undone yet). */
  function newer(h: HistoryEntry): number {
    return app.history.filter((x) => x.id > h.id && x.undoable && !x.undone).length;
  }

  async function undoTo(h: HistoryEntry) {
    const n = newer(h);
    if (n > 0) {
      const ok = await confirm(
        `Undo ${n + 1} changes?`,
        `To undo “${h.label}”, the ${n === 1 ? "change" : `${n} changes`} made after it ${n === 1 ? "is" : "are"} undone first, newest first.`,
        "Undo",
      );
      if (!ok) return;
    }
    await undo(h.id);
  }

  function onKey(e: KeyboardEvent) {
    const meta = e.metaKey || e.ctrlKey;
    if (!meta || e.shiftKey || e.altKey || e.key.toLowerCase() !== "z") return;
    if (app.dialog || app.renameIds || app.showSettings || app.slideshow) return;
    if ((e.target as HTMLElement)?.closest("input, textarea, select, [contenteditable]")) return;
    e.preventDefault();
    undo();
  }
</script>

<svelte:window onkeydown={onKey} />

{#if app.showHistory}
  {@const top = latestUndoable()}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
    <div class="win" role="dialog" aria-modal="true" aria-label="Edit history" tabindex="-1"
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); close(); } }}>
      <header>
        <h3>Edit history</h3>
        <button class="ghost icon" onclick={close} aria-label="Close"><X size={16} /></button>
      </header>
      <p class="small">
        Moves, copies, renames and new folders, newest first. Undo puts files back where they were and never
        overwrites anything. Photos sent to the Trash are restored from the system Trash / Recycle Bin.
      </p>
      <div class="list">
        {#each app.history as h (h.id)}
          {@const Icon = icons[h.kind] ?? Undo2}
          <div class="item" class:undone={h.undone}>
            <span class="icon"><Icon size={14} /></span>
            <div class="what">
              <div class="label" title={h.label}>{h.label}</div>
              <div class="meta">
                {when(h.at)}
                {#if h.undone} · <Check size={11} /> undone{/if}
                {#if !h.undoable} · restore from the Trash if needed{/if}
              </div>
              {#if h.note}<div class="why"><TriangleAlert size={11} /> {h.note}</div>{/if}
            </div>
            {#if h.undoable && !h.undone}
              <button disabled={app.undoing} onclick={() => undoTo(h)}
                title={newer(h) ? `Also undoes the ${newer(h)} newer change${newer(h) === 1 ? "" : "s"}` : "Put this back"}>
                <Undo2 size={13} /> {newer(h) ? "Undo to here" : "Undo"}
              </button>
            {/if}
          </div>
        {:else}
          <p class="small empty">No changes yet. Moves, copies and renames you make show up here.</p>
        {/each}
      </div>
      <div class="buttons">
        <span class="faint small">⌘Z / Ctrl+Z undoes the latest change.</span>
        <span class="spacer"></span>
        <button disabled={!top || app.undoing} onclick={() => undo()}><Undo2 size={13} /> Undo latest</button>
        <button class="primary" onclick={close}>Done</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 1600; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .win { width: 600px; max-width: calc(100vw - 40px); max-height: calc(100vh - 40px); background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 16px 18px; box-shadow: var(--shadow); line-height: 1.45; display: flex; flex-direction: column; }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
  h3 { margin: 0; font-size: 15px; }
  p { margin: 4px 0 10px; color: var(--text-2); }
  .small { font-size: 12px; }
  .empty { padding: 8px 0; }
  .list { overflow-y: auto; min-height: 60px; max-height: 55vh; border: 1px solid var(--line-2); border-radius: 8px; background: var(--bg); padding: 4px 8px; }
  .item { display: flex; gap: 10px; align-items: center; padding: 6px 0; border-bottom: 1px solid var(--line); }
  .item:last-child { border-bottom: none; }
  .item.undone .label { color: var(--text-3); text-decoration: line-through; }
  .icon { color: var(--text-2); display: flex; }
  .what { flex: 1; min-width: 0; }
  .label { font-size: 12.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .meta { font-size: 11.5px; color: var(--text-2); display: flex; align-items: center; gap: 3px; }
  .why { font-size: 11.5px; color: #e0a24a; display: flex; align-items: center; gap: 4px; }
  .buttons { display: flex; gap: 8px; margin-top: 12px; align-items: center; }
  .spacer { flex: 1; }
  .faint { color: var(--text-3); }
</style>
