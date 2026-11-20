<script lang="ts">
  import { ImageOff, Film, CloudOff, ChevronUp, ChevronDown } from "@lucide/svelte";
  import Stars from "./Stars.svelte";
  import EmptyState from "./EmptyState.svelte";
  import StackBadge from "./StackBadge.svelte";
  import { app, selection, clickSelect, openViewer, rate, rootOnline, selectedIds, dropOnView, loadPhotos, isClosedStack } from "../store.svelte";
  import { thumbUrl, labelColor, formatDate, basename, isVideo, type PhotoLite } from "../api";
  import { showMenu } from "../menu.svelte";
  import { photoMenu } from "../actions";
  import { browseKeys, photoDragStart, photoDragEnd, canDropHere, dragGhosts } from "../browse";

  const ROW = 44;
  const ghostCount = $derived(dragGhosts().length);
  let scroller: HTMLDivElement | undefined = $state();
  let height = $state(600);
  let scrollTop = $state(0);
  let dropHint = $state(false);

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 5));
  const last = $derived(Math.min(app.photos.length, Math.ceil((scrollTop + height) / ROW) + 5));

  let lastScope = "";
  $effect(() => {
    const key = JSON.stringify(app.scope);
    if (key !== lastScope && scroller) {
      lastScope = key;
      scroller.scrollTop = 0;
    }
  });

  function reveal(i: number) {
    if (!scroller) return;
    const top = i * ROW;
    if (top < scroller.scrollTop) scroller.scrollTop = top;
    else if (top + ROW > scroller.scrollTop + height) scroller.scrollTop = top + ROW - height;
  }

  function onKey(e: KeyboardEvent) {
    browseKeys(e, { ArrowDown: 1, ArrowUp: -1, ArrowRight: 1, ArrowLeft: -1, PageDown: Math.floor(height / ROW), PageUp: -Math.floor(height / ROW) }, reveal);
  }

  function sortBy(key: string) {
    if (app.sort === key) app.desc = !app.desc;
    else {
      app.sort = key;
      app.desc = key === "date" || key === "rating" || key === "modified" || key === "size";
    }
    loadPhotos();
  }

  function dims(p: PhotoLite) {
    return p.width && p.height ? `${p.width} × ${p.height}` : "";
  }

  const COLS: { key: string | null; label: string }[] = [
    { key: "name", label: "Name" },
    { key: "date", label: "Date taken" },
    { key: "rating", label: "Rating" },
    { key: null, label: "Label" },
    { key: null, label: "Size" },
    { key: null, label: "Type" },
    { key: null, label: "Folder" },
  ];
</script>

<svelte:window onkeydown={onKey} />

<div class="wrap">
  <div class="head row">
    <span class="c-thumb"></span>
    {#each COLS as c, i}
      <button class={`c c${i}`} class:sortable={c.key} class:on={c.key && app.sort === c.key} disabled={!c.key} onclick={() => c.key && sortBy(c.key)}>
        {c.label}
        {#if c.key && app.sort === c.key}{#if app.desc}<ChevronDown size={12} />{:else}<ChevronUp size={12} />{/if}{/if}
      </button>
    {/each}
  </div>

  <div
    class="scroller"
    class:drop={dropHint}
    bind:this={scroller}
    bind:clientHeight={height}
    onscroll={() => (scrollTop = scroller!.scrollTop)}
    onmousedown={(e) => { if (e.target === scroller || (e.target as HTMLElement).classList.contains("canvas")) selection.clear(); }}
    ondragover={(e) => { if (canDropHere(e)) { e.preventDefault(); dropHint = true; } }}
    ondragleave={() => (dropHint = false)}
    ondrop={(e) => { e.preventDefault(); dropHint = false; dropOnView(e); }}
    role="grid"
    tabindex="0"
  >
    <div class="canvas" style:height={`${app.photos.length * ROW}px`}>
      {#each [...dragGhosts(), ...app.photos.slice(first, last)] as p, k (p.id)}
        {@const ghost = k < ghostCount}
        {@const i = ghost ? -200 : first + k - ghostCount}
        {@const sel = selection.has(p.id)}
        {@const offline = !rootOnline(p.root_id)}
        <div
          class="row item"
          class:sel
          class:odd={i % 2 === 1}
          class:focus={app.focusId === p.id}
          class:ghost
          style:transform={`translateY(${i * ROW}px)`}
          role="row"
          tabindex="-1"
          aria-selected={sel}
          draggable="true"
          ondragstart={(e) => photoDragStart(e, p)}
          ondragend={photoDragEnd}
          onmousedown={(e) => { if (e.button === 0 && !(sel && !e.shiftKey && !e.metaKey && !e.ctrlKey)) clickSelect(p.id, e); }}
          onclick={(e) => { if (sel && !e.shiftKey && !e.metaKey && !e.ctrlKey && selection.size > 1) clickSelect(p.id, e); }}
          ondblclick={() => openViewer(p.id)}
          oncontextmenu={(e) => showMenu(e, photoMenu(p))}
          onkeydown={() => {}}
        >
          <span class="c-thumb">
            {#if p.thumb}<img src={thumbUrl(p)} alt="" draggable="false" loading="lazy" />{:else if isVideo(p)}<Film size={16} />{:else}<ImageOff size={16} />{/if}
          </span>
          <span class="c c0 name" class:member={p.stack_id != null && !isClosedStack(p) && !p.stack_top} title={p.path}><span class="fn">{p.filename}</span>{#if p.stack_id != null}<StackBadge {p} small />{/if}{#if offline}<CloudOff size={12} />{/if}</span>
          <span class="c c1">{formatDate(p.taken_at)}</span>
          <span class="c c2"><Stars value={p.rating} size={11} onchange={(r) => rate(selection.has(p.id) ? selectedIds() : [p.id], r)} /></span>
          <span class="c c3">{#if labelColor(p.label)}<span class="dot" style:background={labelColor(p.label)}></span>{p.label}{/if}</span>
          <span class="c c4">{dims(p)}</span>
          <span class="c c5">{p.ext.toUpperCase()}</span>
          <span class="c c6" title={p.folder}>{basename(p.folder)}</span>
        </div>
      {/each}
    </div>
    <EmptyState />
  </div>
</div>

<style>
  .wrap { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .row { display: grid; grid-template-columns: 52px minmax(160px, 2.2fr) 150px 96px 90px 100px 56px minmax(100px, 1.2fr); align-items: center; gap: 0 10px; padding: 0 12px; }
  .head { height: 30px; flex-shrink: 0; border-bottom: 1px solid var(--line); background: var(--bg-2); font-size: 11.5px; color: var(--text-2); }
  .head .c { background: none; border: none; padding: 0; height: 100%; justify-content: flex-start; gap: 3px; color: inherit; border-radius: 0; }
  .head .c:disabled { opacity: 1; }
  .head .sortable:hover { color: var(--text); }
  .head .on { color: var(--text); font-weight: 600; }
  .scroller { flex: 1; overflow-y: auto; overflow-x: hidden; position: relative; outline: none; }
  .scroller.drop { box-shadow: inset 0 0 0 2px var(--accent); background: var(--accent-soft); }
  .canvas { position: relative; }
  .ghost { visibility: hidden; pointer-events: none; }
  .item { position: absolute; left: 0; right: 0; top: 0; height: 44px; font-size: 12.5px; color: var(--text-2); }
  .item.odd { background: color-mix(in srgb, var(--bg-2) 55%, transparent); }
  .item:hover { background: var(--bg-3); }
  .item.sel { background: var(--accent-soft); color: var(--text); }
  .item.focus.sel { box-shadow: inset 2px 0 0 var(--accent); }
  .c { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; display: flex; align-items: center; gap: 6px; }
  .name { color: var(--text); }
  .fn { overflow: hidden; text-overflow: ellipsis; }
  .name.member { padding-left: 14px; }
  .name :global(svg) { color: var(--text-3); flex-shrink: 0; }
  .c-thumb { width: 52px; height: 38px; display: flex; align-items: center; justify-content: center; color: var(--text-3); }
  .c-thumb img { max-width: 52px; max-height: 38px; object-fit: contain; border-radius: 2px; box-shadow: 0 1px 3px rgba(0,0,0,.35); }
  .dot { width: 9px; height: 9px; border-radius: 50%; flex-shrink: 0; }
  @media (max-width: 1100px) { .row { grid-template-columns: 52px minmax(140px, 2fr) 140px 96px 0 0 52px minmax(80px, 1fr); } .c3, .c4 { visibility: hidden; } }
</style>
