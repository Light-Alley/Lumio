<script lang="ts">
  // The little "group" marker on a photo: on a closed group it shows how many shots are
  // inside and opens it; on an open group it closes it again.
  import { Layers, ChevronsLeft } from "@lucide/svelte";
  import { toggleStack, isClosedStack } from "../store.svelte";
  import type { PhotoLite } from "../api";

  let { p, small = false }: { p: PhotoLite; small?: boolean } = $props();
  const closed = $derived(isClosedStack(p));
</script>

{#if p.stack_id != null && (closed || p.stack_top)}
  <button
    class="stack"
    class:open={!closed}
    class:small
    title={closed ? `Group of ${p.stack_count} photos. Click to open (S)` : "Close group (S)"}
    aria-label={closed ? `Open group of ${p.stack_count} photos` : "Close group"}
    onmousedown={(e) => e.stopPropagation()}
    ondblclick={(e) => e.stopPropagation()}
    onclick={(e) => { e.stopPropagation(); toggleStack(p.stack_id!); }}
  >
    {#if closed}<Layers size={small ? 10 : 11} />{p.stack_count}{:else}<ChevronsLeft size={small ? 10 : 11} />{p.stack_count}{/if}
  </button>
{:else if p.stack_id != null}
  <span class="stack member" class:small title="In a group"><Layers size={small ? 10 : 11} /></span>
{/if}

<style>
  .stack {
    display: inline-flex; align-items: center; gap: 3px; height: 18px; padding: 0 6px; border: none; border-radius: 9px;
    background: rgba(0, 0, 0, .66); color: #fff; font: 600 10.5px/1 system-ui, sans-serif; cursor: pointer;
    box-shadow: 0 0 0 1px rgba(255,255,255,.18);
  }
  .stack:hover { background: var(--accent); }
  .stack.open { background: var(--accent); }
  .stack.member { cursor: default; padding: 0 4px; background: rgba(0,0,0,.45); }
  .small { height: 16px; font-size: 9.5px; padding: 0 5px; }
</style>
