<script lang="ts">
  // Help > Keyboard Shortcuts. Single keys such as G or 0–5 can't be shown in the
  // macOS menu bar (they would stop working in text boxes), so they are listed here.
  import { isMac } from "../api";
  import { shortcutsHelp } from "../menubar.svelte";

  let box: HTMLDivElement | undefined = $state();
  $effect(() => { if (shortcutsHelp.open) box?.focus(); });

  const m = isMac ? "⌘" : "Ctrl+";
  const sh = isMac ? "⇧⌘" : "Ctrl+Shift+";
  const groups: [string, [string, string][]][] = [
    ["Anywhere", [
      ["G  L  F  M", "Grid, list, filmstrip, map"],
      ["I", "Show or hide the info panel"],
      ["P", "Slideshow"],
      [`${m}T  ${m}W  ${m}1–9`, "New tab, close tab, switch tab"],
      [`${m}F`, "Search"],
      [`${m}O`, "Add a folder to the library"],
      [`${sh}N`, "New folder in the open folder"],
      [`${m}=  ${m}-`, "Larger or smaller thumbnails"],
      [`${m}Z`, "Undo the latest move, copy or rename"],
      [`${m},`, "Settings"],
      [isMac ? "⌃⌘F" : "F11", "Full screen"],
    ]],
    ["Browsing", [
      ["Arrows  Home  End", "Move through photos (Shift extends the selection)"],
      [`${m}A  Esc`, "Select all, clear selection"],
      ["Space  Enter", "Open in the viewer"],
      ["0–5", "Rating"],
      ["6–9", "Colour label: red, yellow, green, blue"],
      [`${m}G  ${sh}G  S`, "Group, ungroup, open or close group"],
      [`${m}[  ${m}]`, "Rotate left, right"],
      ["F2", "Rename"],
      [`${m}C`, "Copy photos to the clipboard"],
      [isMac ? "⌘⌫" : "Delete", "Move to Trash"],
    ]],
    ["Viewer", [
      ["Arrows", "Previous, next"],
      ["Z  B  F  I", "Zoom, background, full screen, info"],
      ["Esc", "Leave full screen, then close"],
      ["Space/K  J  L  M", "Video: play/pause, back 10 s, forward 10 s, mute"],
    ]],
    ["Slideshow", [
      ["Space  Arrows", "Pause, previous, next"],
      ["+  -  S  F  I  Esc", "Speed, shuffle, full screen, caption, end"],
    ]],
  ];
</script>

{#if shortcutsHelp.open}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && (shortcutsHelp.open = false)}>
    <div class="dialog" bind:this={box} role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" tabindex="-1"
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); shortcutsHelp.open = false; } }}>
      <h3>Keyboard shortcuts</h3>
      <div class="cols">
        {#each groups as [title, rows]}
          <section>
            <h4>{title}</h4>
            {#each rows as [keys, what]}
              <div class="row"><span class="keys">{#each keys.split("  ") as k}<span class="kbd">{k}</span>{/each}</span><span>{what}</span></div>
            {/each}
          </section>
        {/each}
      </div>
      <div class="buttons"><button class="primary" onclick={() => (shortcutsHelp.open = false)}>Done</button></div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 2100; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .dialog { width: 820px; max-width: calc(100vw - 40px); max-height: calc(100vh - 60px); overflow-y: auto; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 18px 20px; box-shadow: var(--shadow); }
  h3 { margin: 0 0 12px; font-size: 15px; }
  h4 { margin: 0 0 6px; font-size: 12px; text-transform: uppercase; letter-spacing: .04em; color: var(--text-3); }
  .cols { display: grid; grid-template-columns: 1fr 1fr; gap: 16px 28px; }
  .row { display: grid; grid-template-columns: 150px 1fr; gap: 10px; padding: 3px 0; font-size: 12.5px; line-height: 1.4; }
  .keys { display: flex; flex-wrap: wrap; gap: 3px; }
  .buttons { display: flex; justify-content: flex-end; margin-top: 14px; }
</style>
