<script lang="ts">
  import MenuList from "./MenuList.svelte";
  import { ChevronRight, Check, Loader } from "@lucide/svelte";
  import { closeMenu, type MenuItem } from "../menu.svelte";

  let { items, x, y }: { items: MenuItem[]; x: number; y: number } = $props();
  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ left: 0, top: 0 });
  let openSub = $state<number | null>(null);
  let subItems = $state<MenuItem[] | null>(null);
  let subPos = $state({ x: 0, y: 0 });
  let loading = $state(false);

  $effect(() => {
    if (!el) return;
    const r = el.getBoundingClientRect();
    pos = {
      left: Math.max(4, Math.min(x, window.innerWidth - r.width - 6)),
      top: Math.max(4, Math.min(y, window.innerHeight - r.height - 6)),
    };
  });

  async function enter(i: number, item: MenuItem, ev: MouseEvent) {
    if (item.separator || !item.submenu) { openSub = null; return; }
    if (openSub === i) return;
    openSub = i;
    const r = (ev.currentTarget as HTMLElement).getBoundingClientRect();
    subPos = { x: r.right - 2, y: r.top - 5 };
    if (typeof item.submenu === "function") {
      subItems = null;
      loading = true;
      const fn = item.submenu;
      const res = await fn();
      if (openSub === i) { subItems = res; loading = false; }
    } else {
      subItems = item.submenu;
    }
  }
</script>

<div class="menu" bind:this={el} style:left={`${pos.left}px`} style:top={`${pos.top}px`} role="menu" tabindex="-1">
  {#each items as item, i}
    {#if item.separator}
      <div class="sep"></div>
    {:else}
      <button
        class="item"
        class:danger={item.danger}
        class:hl={openSub === i}
        disabled={item.disabled}
        role="menuitem"
        onmouseenter={(e) => enter(i, item, e)}
        onclick={(e) => { if (item.submenu) { enter(i, item, e); return; } closeMenu(); item.action?.(); }}
      >
        <span class="check">{#if item.checked}<Check size={13} />{:else if item.color}<span class="dot" style:background={item.color}></span>{/if}</span>
        <span class="label">{item.label}</span>
        {#if item.shortcut}<span class="sc">{item.shortcut}</span>{/if}
        {#if item.submenu}<ChevronRight size={13} />{/if}
      </button>
    {/if}
  {/each}
</div>
{#if openSub !== null}
  {#if subItems}
    <MenuList items={subItems.length ? subItems : [{ label: "No applications found", disabled: true }]} x={subPos.x} y={subPos.y} />
  {:else if loading}
    <div class="menu" style:left={`${subPos.x}px`} style:top={`${subPos.y}px`}><div class="item"><Loader size={13} /> Loading…</div></div>
  {/if}
{/if}

<style>
  .menu {
    position: fixed; z-index: 1000; min-width: 210px; max-width: 340px; max-height: 80vh; overflow-y: auto;
    background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 8px; padding: 5px; box-shadow: var(--shadow);
  }
  .item {
    width: 100%; display: flex; align-items: center; gap: 6px; border: none; background: none;
    padding: 5px 8px; border-radius: 5px; text-align: left; color: var(--text);
  }
  .item:hover:not(:disabled), .item.hl { background: var(--accent) !important; color: white; }
  .item.danger { color: var(--danger); }
  .item.danger:hover { color: white; background: var(--danger) !important; }
  .label { flex: 1; overflow: hidden; text-overflow: ellipsis; }
  .sc { color: var(--text-3); font-size: 11px; }
  .item:hover .sc { color: rgba(255,255,255,.8); }
  .check { width: 14px; display: inline-flex; justify-content: center; }
  .dot { width: 10px; height: 10px; border-radius: 50%; display: inline-block; }
  .sep { height: 1px; background: var(--line); margin: 4px 6px; }
</style>
