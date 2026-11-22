<script lang="ts">
  // The thin strip on the folder sidebar's right edge. Drag it to make the sidebar
  // narrower or wider; double-click to go back to the standard width. The width is
  // kept between a usable minimum and 40% of the window, and remembered.
  import { app, clampSidebar, saveSetting, SIDEBAR_DEFAULT, SIDEBAR_MIN, SIDEBAR_MAX } from "../store.svelte";

  let dragging = $state(false);
  let startX = 0;
  let startW = 0;

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    dragging = true;
    startX = e.clientX;
    startW = app.sidebarWidth;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!dragging) return;
    app.sidebarWidth = clampSidebar(startW + e.clientX - startX);
  }
  function up() {
    if (!dragging) return;
    dragging = false;
    saveSetting("sidebar_width", String(app.sidebarWidth));
  }
  function reset() {
    app.sidebarWidth = clampSidebar(SIDEBAR_DEFAULT);
    saveSetting("sidebar_width", String(app.sidebarWidth));
  }
  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? 40 : 10;
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      app.sidebarWidth = clampSidebar(app.sidebarWidth + (e.key === "ArrowLeft" ? -step : step));
      saveSetting("sidebar_width", String(app.sidebarWidth));
    }
  }
</script>

<svelte:window onresize={() => (app.sidebarWidth = clampSidebar(app.sidebarWidth))} />

<!-- A slider: it can be focused and moved with the arrow keys, and screen readers announce the width. -->
<div
  class="resizer"
  class:dragging
  role="slider"
  aria-orientation="horizontal"
  aria-label="Folder sidebar width"
  aria-valuenow={app.sidebarWidth}
  aria-valuemin={SIDEBAR_MIN}
  aria-valuemax={SIDEBAR_MAX}
  tabindex="0"
  title="Drag to resize. Double-click for the standard width."
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  ondblclick={reset}
  onkeydown={key}
></div>
{#if dragging}<div class="cover"></div>{/if}

<style>
  .resizer {
    width: 7px; margin: 0 -4px 0 -3px; flex-shrink: 0; cursor: col-resize; position: relative; z-index: 5;
    background: transparent; transition: background 0.12s;
  }
  .resizer:hover, .resizer.dragging, .resizer:focus-visible { background: var(--accent-soft); outline: none; }
  .resizer::after { content: ""; position: absolute; top: 0; bottom: 0; left: 3px; width: 1px; }
  .resizer:hover::after, .resizer.dragging::after { background: var(--accent); }
  /* Keeps the cursor and stops text selection while dragging over other parts of the window. */
  .cover { position: fixed; inset: 0; cursor: col-resize; z-index: 4; user-select: none; -webkit-user-select: none; }
</style>
