<script lang="ts">
  // Editor for a smart collection: a name and a list of rules. Shows how many photos
  // match as you type, counted by the same Rust code that fills the collection.
  import { tick } from "svelte";
  import { X, Plus, Trash2, FolderOpen } from "@lucide/svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { api, errText, type Rule, type RuleSet } from "../api";
  import { app, saveCollection } from "../store.svelte";
  import { FIELDS, TEMPLATES, fieldOf, opOf, newRule, startValue } from "../collections";

  let name = $state("");
  let rules = $state<RuleSet>({ any: false, rules: [] });
  let count = $state<number | null>(null);
  let error = $state("");
  let busy = $state(false);
  let nameEl: HTMLInputElement | undefined = $state();

  const editing = $derived(app.collectionEdit);
  const isNew = $derived(editing?.id == null);

  let opened = false;
  $effect(() => {
    const ed = app.collectionEdit;
    if (ed && !opened) {
      opened = true;
      name = ed.name;
      rules = { any: ed.rules.any, rules: ed.rules.rules.length ? ed.rules.rules.map((r) => ({ ...r })) : [newRule()] };
      error = "";
      count = null;
      tick().then(() => nameEl?.focus());
    } else if (!ed) opened = false;
  });

  // Live count, shortly after any change.
  let seq = 0;
  let timer: ReturnType<typeof setTimeout>;
  $effect(() => {
    const set = $state.snapshot(rules) as RuleSet;
    if (!app.collectionEdit) return;
    clearTimeout(timer);
    const mine = ++seq;
    timer = setTimeout(async () => {
      try {
        const n = await api.countRules(set);
        if (mine !== seq) return;
        count = n;
        error = "";
      } catch (e) {
        if (mine !== seq) return;
        count = null;
        error = errText(e);
      }
    }, 150);
  });

  function setField(r: Rule, field: string) {
    const fresh = newRule(field);
    r.field = fresh.field;
    r.op = fresh.op;
    r.value = fresh.value;
    r.value2 = "";
    if (field === "folder" && app.scope.kind === "folder") r.value = app.scope.path;
  }

  function setOp(r: Rule, op: string) {
    const before = opOf(r);
    r.op = op;
    const after = opOf(r);
    // Keep what was typed when the kind of value stays the same.
    if (before.value !== after.value || after.value === "choice" || after.value === "none") r.value = startValue(after);
    if (!after.between) r.value2 = "";
  }

  async function chooseFolder(r: Rule) {
    const p = (await openDialog({ directory: true, multiple: false, title: "Choose a folder", defaultPath: r.value || undefined })) as string | null;
    if (p) r.value = p;
  }

  function useTemplate(t: (typeof TEMPLATES)[number]) {
    if (!name.trim() || TEMPLATES.some((x) => x.name === name)) name = t.name;
    rules = { any: t.rules.any, rules: t.rules.rules.map((r) => ({ ...r })) };
  }

  async function save() {
    if (busy || !editing) return;
    if (!name.trim()) { error = "Give the collection a name."; nameEl?.focus(); return; }
    busy = true;
    const ok = await saveCollection(editing.id, name, $state.snapshot(rules) as RuleSet);
    busy = false;
    if (!ok) nameEl?.focus();
  }

  function close() {
    if (!busy) app.collectionEdit = null;
  }
</script>

{#if editing}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
    <div class="win" role="dialog" aria-modal="true" aria-label={isNew ? "New smart collection" : "Edit smart collection"} tabindex="-1"
      onkeydown={(e) => {
        if (e.key === "Escape") { e.stopPropagation(); close(); }
        if (e.key === "Enter" && (e.target as HTMLElement).tagName === "INPUT") { e.preventDefault(); save(); }
      }}>
      <header>
        <h3>{isNew ? "New smart collection" : "Edit smart collection"}</h3>
        <button class="ghost icon" onclick={close} aria-label="Close"><X size={16} /></button>
      </header>

      <label class="field"><span>Name</span>
        <input bind:this={nameEl} bind:value={name} placeholder="Best of 2024" spellcheck="false" />
      </label>

      {#if isNew}
        <div class="templates">
          <span class="faint small">Start from:</span>
          {#each TEMPLATES as t}<button class="chip" onclick={() => useTemplate(t)}>{t.name}</button>{/each}
        </div>
      {/if}

      <div class="match">
        Show photos that match
        <select bind:value={rules.any} aria-label="Match all or any">
          <option value={false}>all</option>
          <option value={true}>any</option>
        </select>
        of these rules:
      </div>

      <div class="rules">
        {#each rules.rules as r, i (i)}
          {@const f = fieldOf(r)}
          {@const o = opOf(r)}
          <div class="rule">
            <select class="fieldsel" value={r.field} onchange={(e) => setField(r, e.currentTarget.value)} aria-label="What to look at">
              {#each FIELDS as x}<option value={x.field}>{x.label}</option>{/each}
            </select>
            <select class="opsel" value={r.op} onchange={(e) => setOp(r, e.currentTarget.value)} aria-label="Condition">
              {#each f.ops as x}<option value={x.op}>{x.label}</option>{/each}
            </select>
            <div class="value">
              {#if o.value === "stars"}
                <select bind:value={r.value} aria-label="Stars">
                  {#each [0, 1, 2, 3, 4, 5] as n}<option value={String(n)}>{n === 0 ? "No stars" : "★".repeat(n)}</option>{/each}
                </select>
              {:else if o.value === "choice"}
                <select bind:value={r.value} aria-label="Value">
                  {#each o.choices ?? [] as [v, l]}<option value={v}>{l}</option>{/each}
                </select>
              {:else if o.value === "date"}
                <input type="date" bind:value={r.value} aria-label="Date" />
                {#if o.between}<span class="faint">and</span><input type="date" bind:value={r.value2} aria-label="Second date" />{/if}
              {:else if o.value === "folder"}
                <input bind:value={r.value} placeholder="Choose a folder" spellcheck="false" title={r.value} aria-label="Folder" />
                <button class="ghost icon" onclick={() => chooseFolder(r)} title="Choose a folder"><FolderOpen size={15} /></button>
              {:else if o.value === "number"}
                <input type="number" bind:value={r.value} placeholder={o.placeholder} aria-label="Number" />
              {:else if o.value === "text"}
                <input bind:value={r.value} placeholder={o.placeholder} spellcheck="false" aria-label="Text" />
              {/if}
            </div>
            <button class="ghost icon" onclick={() => rules.rules.splice(i, 1)} title="Remove this rule" aria-label="Remove rule" disabled={rules.rules.length === 1 && !r.value && o.value !== "none"}>
              <Trash2 size={14} />
            </button>
          </div>
        {/each}
        <button class="add" onclick={() => rules.rules.push(newRule(rules.rules.length ? "taken" : "rating"))}><Plus size={14} /> Add rule</button>
      </div>

      <p class="status small" aria-live="polite">
        {#if error}<span class="err">{error}</span>
        {:else if count !== null}{count.toLocaleString()} photo{count === 1 ? "" : "s"} match right now.
        {/if}
      </p>
      <p class="faint small">A smart collection keeps itself up to date: photos join and leave as you rate, label, add or move them. Nothing is copied or moved.</p>

      <div class="buttons">
        <button onclick={close} disabled={busy}>Cancel</button>
        <button class="primary" onclick={save} disabled={busy || !!error}>{isNew ? "Create collection" : "Save"}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 1600; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .win { width: 660px; max-width: calc(100vw - 40px); max-height: calc(100vh - 40px); overflow-y: auto; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 16px 18px; box-shadow: var(--shadow); line-height: 1.45; display: flex; flex-direction: column; }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
  h3 { margin: 0; font-size: 15px; }
  p { margin: 6px 0; }
  .small { font-size: 11.5px; }
  .field input { width: 100%; box-sizing: border-box; }
  .templates { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin: -2px 0 12px; }
  .chip { font-size: 11.5px; padding: 2px 9px; border-radius: 999px; background: var(--bg-4); border: 1px solid var(--line-2); }
  .chip:hover { border-color: var(--accent); }
  .match { display: flex; align-items: center; gap: 6px; margin-bottom: 8px; color: var(--text-2); }
  .match select { width: auto; }
  .rules { border: 1px solid var(--line-2); border-radius: 8px; background: var(--bg); padding: 8px; display: flex; flex-direction: column; gap: 6px; }
  .rule { display: flex; gap: 6px; align-items: center; }
  .rule input { box-sizing: border-box; }
  .fieldsel { flex: 0 0 150px; }
  .opsel { flex: 0 0 170px; }
  .value { flex: 1; display: flex; gap: 6px; align-items: center; min-width: 0; }
  .value input, .value select { flex: 1; min-width: 0; width: auto; }
  .add { align-self: flex-start; display: flex; gap: 4px; align-items: center; font-size: 12px; padding: 3px 10px; }
  .status { min-height: 1.45em; margin-top: 10px; }
  .err { color: var(--danger); }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 10px; }
</style>
