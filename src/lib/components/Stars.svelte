<script lang="ts">
  import { Star } from "@lucide/svelte";
  let { value = 0, size = 14, onchange, mixed = false }: { value?: number; size?: number; onchange?: (v: number) => void; mixed?: boolean } = $props();
  let hover = $state(0);
</script>

<div class="stars" class:interactive={!!onchange} class:mixed role="radiogroup" aria-label="Rating" onmouseleave={() => (hover = 0)}>
  {#each [1, 2, 3, 4, 5] as n}
    <button
      type="button"
      class="star"
      class:on={(hover || value) >= n}
      aria-label={`${n} star${n > 1 ? "s" : ""}`}
      disabled={!onchange}
      onmouseenter={() => onchange && (hover = n)}
      onclick={(e) => { e.stopPropagation(); onchange?.(value === n ? 0 : n); }}
    >
      <Star {size} fill={(hover || value) >= n ? "currentColor" : "none"} strokeWidth={1.6} />
    </button>
  {/each}
</div>

<style>
  .stars { display: inline-flex; gap: 1px; }
  .star { background: none; border: none; padding: 1px; color: var(--text-3); }
  .star:hover:not(:disabled) { background: none; }
  .star:disabled { opacity: 1; }
  .star.on { color: var(--star); }
  .mixed .star:not(.on) { color: var(--line-2); }
</style>
