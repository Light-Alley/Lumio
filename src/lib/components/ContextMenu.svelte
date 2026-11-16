<script lang="ts">
  import MenuList from "./MenuList.svelte";
  import { menu, closeMenu } from "../menu.svelte";
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && closeMenu()} onblur={closeMenu} onresize={closeMenu} />
{#if menu.open}
  <div class="backdrop" role="presentation" onmousedown={closeMenu} oncontextmenu={(e) => { e.preventDefault(); closeMenu(); }}></div>
  <MenuList items={menu.items} x={menu.x} y={menu.y} />
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 999; }
</style>
