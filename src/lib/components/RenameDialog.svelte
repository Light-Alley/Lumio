<script lang="ts">
  // Batch rename: build new names from a pattern, or find and replace text in the current names.
  // The preview comes from the same Rust code that does the renaming, so what you see is what you get.
  import { tick } from "svelte";
  import { X, TriangleAlert } from "@lucide/svelte";
  import { api, errText, type PlannedName, type RenameOptions, type NameCase } from "../api";
  import { app, loadHistory, loadPhotos, offerUndo, toast } from "../store.svelte";
  import { remember, keep } from "../publish.svelte";

  const TOKENS: [string, string][] = [
    ["{name}", "Current name"],
    ["{n}", "Number"],
    ["{date}", "Date taken"],
    ["{time}", "Time"],
    ["{year}", "Year"],
    ["{month}", "Month"],
    ["{day}", "Day"],
    ["{folder}", "Folder"],
    ["{camera}", "Camera"],
    ["{title}", "Title"],
  ];
  const CASES: [NameCase, string][] = [["keep", "As typed"], ["lower", "lowercase"], ["upper", "UPPERCASE"]];
  const EXT_CASES: [NameCase, string][] = [["keep", "Unchanged"], ["lower", ".jpg (lowercase)"], ["upper", ".JPG (uppercase)"]];

  const defaults: RenameOptions = {
    mode: "pattern", pattern: "{date} {n}", start: 1, digits: 3,
    find: "", replace: "", match_case: false, case: "keep", ext_case: "keep", keep_pairs: true,
  };
  let o = $state<RenameOptions>({ ...defaults, ...remember<Partial<RenameOptions>>("rename.options", {}) });
  let plan = $state<PlannedName[]>([]);
  let error = $state("");
  let busy = $state(false);
  let patternEl: HTMLInputElement | undefined = $state();
  let findEl: HTMLInputElement | undefined = $state();

  const ids = $derived(app.renameIds ?? []);
  const changing = $derived(plan.filter((p) => !p.problem && p.new !== p.old));
  const problems = $derived(plan.filter((p) => p.problem));
  const shown = $derived(plan.slice(0, 300));

  let opened = false;
  $effect(() => {
    if (app.renameIds && !opened) {
      opened = true;
      error = "";
      plan = [];
      tick().then(() => (o.mode === "pattern" ? patternEl : findEl)?.focus());
    } else if (!app.renameIds) opened = false;
  });

  // Refresh the preview shortly after any change.
  let seq = 0;
  let timer: ReturnType<typeof setTimeout>;
  $effect(() => {
    const opts = $state.snapshot(o) as RenameOptions;
    const list = ids;
    if (!list.length) return;
    clearTimeout(timer);
    const mine = ++seq;
    timer = setTimeout(async () => {
      try {
        const r = await api.renamePreview(list, { ...opts, start: Number(opts.start) || 0, digits: Number(opts.digits) || 1 });
        if (mine !== seq) return;
        plan = r;
        error = "";
      } catch (e) {
        if (mine !== seq) return;
        plan = [];
        error = errText(e);
      }
    }, 120);
  });

  function insert(token: string) {
    const el = patternEl;
    if (!el) { o.pattern += token; return; }
    const a = el.selectionStart ?? o.pattern.length;
    const b = el.selectionEnd ?? a;
    o.pattern = o.pattern.slice(0, a) + token + o.pattern.slice(b);
    tick().then(() => { el.focus(); el.setSelectionRange(a + token.length, a + token.length); });
  }

  async function run() {
    if (!changing.length || busy) return;
    busy = true;
    keep("rename.options", $state.snapshot(o));
    try {
      const r = await api.renamePhotos(changing.map((p) => [p.id, p.new] as [number, string]));
      app.renameIds = null;
      await loadPhotos();
      app.folderVersion++;
      if (r.failed.length) {
        toast(`${r.done} renamed, ${r.failed.length} could not be.\n${r.failed[0][0]}: ${r.failed[0][1]}`, "error");
      }
      loadHistory();
      if (r.done) offerUndo(r.history, r.done === 1 ? "Renamed 1 photo." : `Renamed ${r.done} photos.`);
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  function close() {
    if (!busy) app.renameIds = null;
  }
</script>

{#if app.renameIds}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
    <div class="win" role="dialog" aria-modal="true" aria-label="Rename photos" tabindex="-1"
      onkeydown={(e) => {
        if (e.key === "Escape") { e.stopPropagation(); close(); }
        if (e.key === "Enter" && (e.target as HTMLElement).tagName === "INPUT") { e.preventDefault(); run(); }
      }}>
      <header>
        <h3>Rename {ids.length} photos</h3>
        <button class="ghost icon" onclick={close} aria-label="Close"><X size={16} /></button>
      </header>

      <div class="seg mode">
        <button class:active={o.mode === "pattern"} onclick={() => (o.mode = "pattern")}>New names from a pattern</button>
        <button class:active={o.mode === "replace"} onclick={() => (o.mode = "replace")}>Find and replace</button>
      </div>

      {#if o.mode === "pattern"}
        <label class="field"><span>Pattern</span>
          <input bind:this={patternEl} bind:value={o.pattern} spellcheck="false" placeholder="{'{date}'} Holiday {'{n}'}" />
        </label>
        <div class="tokens">
          {#each TOKENS as [t, label]}<button class="chip" onclick={() => insert(t)} title={`Insert ${t}`}>{label}</button>{/each}
        </div>
        <div class="row">
          <label class="field small"><span>Start at</span><input type="number" bind:value={o.start} /></label>
          <label class="field small"><span>Digits</span>
            <select bind:value={o.digits}>{#each [1, 2, 3, 4, 5, 6] as d}<option value={d}>{d} ({String(Number(o.start) || 0).padStart(d, "0")})</option>{/each}</select>
          </label>
        </div>
        <label class="check"><input type="checkbox" bind:checked={o.keep_pairs} />
          <span>Keep RAW + JPEG pairs together (files with the same name in the same folder get the same number)</span></label>
      {:else}
        <div class="row">
          <label class="field"><span>Find</span><input bind:this={findEl} bind:value={o.find} spellcheck="false" placeholder="IMG_" /></label>
          <label class="field"><span>Replace with</span><input bind:value={o.replace} spellcheck="false" placeholder="Leave empty to remove" /></label>
        </div>
        <label class="check"><input type="checkbox" bind:checked={o.match_case} /><span>Match upper and lower case exactly</span></label>
      {/if}

      <div class="row">
        <label class="field small"><span>Name case</span>
          <select bind:value={o.case}>{#each CASES as [v, l]}<option value={v}>{l}</option>{/each}</select>
        </label>
        <label class="field small"><span>Extension</span>
          <select bind:value={o.ext_case}>{#each EXT_CASES as [v, l]}<option value={v}>{l}</option>{/each}</select>
        </label>
      </div>

      <div class="preview" aria-label="Preview">
        {#if error}
          <p class="err">{error}</p>
        {:else}
          {#each shown as p (p.id)}
            <div class="line" class:bad={!!p.problem} class:same={!p.problem && p.new === p.old}>
              <span class="old" title={p.old}>{p.old}</span>
              <span class="arrow">→</span>
              <span class="new" title={p.problem ?? p.new}>{p.new}</span>
              {#if p.problem}<span class="why"><TriangleAlert size={12} /> {p.problem}</span>{/if}
            </div>
          {/each}
          {#if plan.length > shown.length}<p class="faint small">…and {plan.length - shown.length} more</p>{/if}
        {/if}
      </div>

      <p class="faint small">Ratings, labels, metadata, groups and thumbnails stay with each photo, and XMP sidecars are renamed with it. You can undo it any time from Edit history (⌘/Ctrl+Z undoes the latest change).</p>
      <p class="warn small" aria-live="polite">
        {#if problems.length && !error}
          {problems.length === 1 ? "1 photo" : `${problems.length} photos`} can't take the new name and will be left as they are.
          {#if o.mode === "pattern" && !/\{\s*(n|#)\s*\}/i.test(o.pattern) && problems.some((p) => p.problem?.startsWith("Same new name"))}
            Add a Number to give each photo its own name.
          {/if}
        {/if}
      </p>

      <div class="buttons">
        <button onclick={close} disabled={busy}>Cancel</button>
        <button class="primary" onclick={run} disabled={busy || !changing.length || !!error}>
          {busy ? "Renaming…" : changing.length === 1 ? "Rename 1 photo" : `Rename ${changing.length} photos`}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 1600; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .win { width: 620px; max-width: calc(100vw - 40px); max-height: calc(100vh - 40px); overflow-y: auto; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 16px 18px; box-shadow: var(--shadow); line-height: 1.45; display: flex; flex-direction: column; }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
  h3 { margin: 0; font-size: 15px; }
  p { margin: 6px 0; }
  .small { font-size: 11.5px; }
  .mode { display: flex; gap: 6px; align-self: flex-start; margin-bottom: 12px; }
  .row { display: flex; gap: 12px; }
  .row > .field { flex: 1; }
  .row > .field.small { flex: 0 1 180px; }
  .field input:not([type]), .field input[type="number"] { width: 100%; box-sizing: border-box; }
  .tokens { display: flex; flex-wrap: wrap; gap: 6px; margin: -2px 0 10px; }
  .chip { font-size: 11.5px; padding: 2px 9px; border-radius: 999px; background: var(--bg-4); border: 1px solid var(--line-2); }
  .chip:hover { border-color: var(--accent); }
  .check { display: flex; gap: 8px; align-items: flex-start; margin: 2px 0 10px; }
  .check input { margin-top: 3px; accent-color: var(--accent); }
  .preview { border: 1px solid var(--line-2); border-radius: 8px; background: var(--bg); padding: 6px 8px; height: 200px; min-height: 120px; overflow-y: auto; font-size: 12px; font-variant-numeric: tabular-nums; margin: 4px 0 8px; user-select: text; -webkit-user-select: text; }
  .line { display: grid; grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr); gap: 8px; padding: 2px 0; align-items: baseline; }
  .line .old, .line .new { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .line .old { color: var(--text-2); }
  .line .arrow { opacity: .5; }
  .line.same .new { opacity: .5; }
  .line.bad .new { color: var(--danger); text-decoration: line-through; }
  .line .why { grid-column: 1 / -1; color: var(--danger); font-size: 11px; display: flex; gap: 4px; align-items: center; margin: -1px 0 2px; }
  .err { color: var(--danger); white-space: pre-wrap; }
  .warn { color: var(--star); min-height: 1.45em; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 10px; }
</style>
