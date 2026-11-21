<script lang="ts">
  // Find duplicate photos and review them. Nothing is removed until you tick photos and
  // press the button; one photo in every group always stays, and removed photos go to the
  // Trash, so they can be put back.
  import { onDestroy } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { X, FolderSearch, CloudOff, Check } from "@lucide/svelte";
  import Stars from "./Stars.svelte";
  import { api, errText, thumbUrl, formatBytes, formatDate, fileManagerName, type DupReport, type DupGroup, type DupMember } from "../api";
  import { app, trashPhotos, toast, errorToast } from "../store.svelte";
  import { remember, keep } from "../publish.svelte";

  const STRICTNESS: [number, string][] = [
    [4, "Same picture, resized or saved again"],
    [8, "Very similar"],
    [12, "Similar, such as burst shots"],
  ];
  const PAGE = 40;

  let mode = $state<"exact" | "similar">(remember("dupes.mode", "exact"));
  let distance = $state<number>(remember("dupes.distance", 4));
  let running = $state(false);
  let progress = $state({ done: 0, total: 0, message: "" });
  let report = $state<DupReport | null>(null);
  let groups = $state<DupGroup[]>([]);
  let shown = $state(PAGE);
  let error = $state("");
  let busy = $state(false);
  /** Photos ticked for the Trash. */
  let ticked = $state(new Set<number>());
  /** Photos moved to the Trash from this search. */
  let trashed = $state(0);

  const where = $derived(app.dupes?.folder ? `in “${app.dupes.name}” and its subfolders` : "in your whole library");
  const tickedCount = $derived(ticked.size);
  const tickedBytes = $derived(groups.flatMap((g) => g.members).filter((m) => ticked.has(m.photo.id)).reduce((n, m) => n + m.photo.size, 0));
  const extraCount = $derived(groups.reduce((n, g) => n + g.members.length - 1, 0));

  let unlisten: UnlistenFn | undefined;
  listen<typeof progress>("duplicates-progress", (e) => (progress = e.payload)).then((u) => (unlisten = u));
  onDestroy(() => unlisten?.());

  let opened = false;
  $effect(() => {
    if (app.dupes && !opened) {
      opened = true;
      report = null;
      groups = [];
      ticked = new Set();
      error = "";
    } else if (!app.dupes) opened = false;
  });

  async function run() {
    if (running || !app.dupes) return;
    keep("dupes.mode", mode);
    keep("dupes.distance", distance);
    running = true;
    error = "";
    report = null;
    groups = [];
    ticked = new Set();
    trashed = 0;
    progress = { done: 0, total: 0, message: "Starting" };
    try {
      const r = await api.findDuplicates({ mode, distance, folder: app.dupes.folder });
      if (!app.dupes) return;
      if (r.cancelled) { error = "Stopped. Nothing was changed."; return; }
      report = r;
      groups = r.groups;
      shown = PAGE;
      // Exact copies: tick every copy except the suggested one. Look-alikes may be
      // different shots, so they start unticked and you choose.
      if (mode === "exact") ticked = new Set(r.groups.flatMap((g) => g.members.filter((m) => !m.keep && m.online).map((m) => m.photo.id)));
    } catch (e) {
      error = errText(e);
    } finally {
      running = false;
    }
  }

  function toggle(g: DupGroup, m: DupMember) {
    if (!m.online) return;
    const next = new Set(ticked);
    if (next.has(m.photo.id)) next.delete(m.photo.id);
    else {
      // Never let a whole group go.
      if (g.members.every((x) => x.photo.id === m.photo.id || next.has(x.photo.id))) {
        toast("One photo in each group always stays. Untick another one first.");
        return;
      }
      next.add(m.photo.id);
    }
    ticked = next;
  }

  function keepOnly(g: DupGroup, m: DupMember) {
    const next = new Set(ticked);
    for (const x of g.members) {
      if (x.photo.id === m.photo.id) next.delete(x.photo.id);
      else if (x.online) next.add(x.photo.id);
    }
    ticked = next;
  }

  function keepAll(g: DupGroup) {
    const next = new Set(ticked);
    for (const x of g.members) next.delete(x.photo.id);
    ticked = next;
  }

  function tickSuggested() {
    ticked = new Set(groups.flatMap((g) => g.members.filter((m) => !m.keep && m.online).map((m) => m.photo.id)));
  }

  async function trash() {
    // Re-check the rule that something stays in every group before anything is touched.
    if (groups.some((g) => g.members.every((m) => ticked.has(m.photo.id)))) {
      error = "Every group must keep at least one photo.";
      return;
    }
    const ids = [...ticked];
    busy = true;
    try {
      const done = await trashPhotos(ids, "Only the ticked photos go, with their XMP sidecars. The others in each group stay. You can restore them from the Trash / Recycle Bin.");
      if (!done) return;
      // What is no longer in the catalogue was moved to the Trash.
      const left = new Set((await api.getPhotos(ids)).map((p) => p.id));
      const gone = new Set(ids.filter((id) => !left.has(id)));
      groups = groups
        .map((g) => ({ members: g.members.filter((m) => !gone.has(m.photo.id)) }))
        .filter((g) => g.members.length > 1);
      ticked = new Set([...ticked].filter((id) => left.has(id)));
      trashed += gone.size;
    } catch (e) {
      errorToast(e);
    } finally {
      busy = false;
    }
  }

  function close() {
    if (running) api.cancelDuplicates();
    if (!busy) app.dupes = null;
  }

  /** The last two folders of a path ("Trips/Rome"); the full path is in the tooltip. */
  const shortFolder = (f: string) => {
    const parts = f.split(/[\\/]/).filter(Boolean);
    return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : f;
  };
  const dims = (m: DupMember) => (m.photo.width && m.photo.height ? `${m.photo.width} × ${m.photo.height}` : "");
</script>

{#if app.dupes}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && !running && close()}>
    <div class="win" class:tall={groups.length > 0} role="dialog" aria-modal="true" aria-label="Find duplicates" tabindex="-1"
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); close(); } }}>
      <header>
        <h3>Find duplicates <span class="faint where">{where}</span></h3>
        <button class="ghost icon" onclick={close} aria-label="Close"><X size={16} /></button>
      </header>

      <div class="options">
        <div class="seg">
          <button class:active={mode === "exact"} onclick={() => (mode = "exact")} disabled={running}>Exact copies</button>
          <button class:active={mode === "similar"} onclick={() => (mode = "similar")} disabled={running}>Look-alikes</button>
        </div>
        {#if mode === "similar"}
          <select bind:value={distance} disabled={running} aria-label="How alike">
            {#each STRICTNESS as [d, l]}<option value={d}>{l}</option>{/each}
          </select>
        {/if}
        <div class="grow"></div>
        {#if running}
          <button onclick={() => api.cancelDuplicates()}>Stop</button>
        {:else}
          <button class="primary" onclick={run}><FolderSearch size={14} /> {report ? "Search again" : "Find duplicates"}</button>
        {/if}
      </div>
      <p class="faint small">
        {#if mode === "exact"}
          Finds files with exactly the same contents, whatever their names. Only files of the same size are read, so this is quick even on a network drive. Drives that aren't connected are skipped.
        {:else}
          Finds the same picture saved more than once (resized, re-compressed or exported) by comparing thumbnails, so it works with drives unplugged. RAW + JPEG pairs of the same shot are not counted.
        {/if}
      </p>

      {#if running}
        <div class="progress" aria-live="polite">
          <div class="bar"><div style:width={progress.total ? `${Math.min(100, (100 * progress.done) / progress.total)}%` : "8%"}></div></div>
          <span class="small faint">{progress.message}{progress.total ? ` · ${progress.done.toLocaleString()} of ${progress.total.toLocaleString()}` : "…"}</span>
        </div>
      {/if}
      {#if error}<p class="err small">{error}</p>{/if}

      {#if report}
        <div class="summary">
          {#if !groups.length && trashed}
            <p><Check size={14} /> {trashed.toLocaleString()} photo{trashed === 1 ? "" : "s"} moved to the Trash. Nothing left to review.</p>
          {:else if !groups.length}
            <p><Check size={14} /> No {mode === "exact" ? "exact copies" : "look-alikes"} found among {report.checked.toLocaleString()} photos.</p>
          {:else}
            <p>
              <strong>{groups.length.toLocaleString()} group{groups.length === 1 ? "" : "s"}</strong>, {extraCount.toLocaleString()} extra photo{extraCount === 1 ? "" : "s"}{mode === "exact" && report.extra_bytes ? ` (about ${formatBytes(report.extra_bytes)})` : ""}.
              The suggested photo to keep in each group is marked <span class="keepword">Keep</span>.
            </p>
          {/if}
          {#if report.skipped}
            <p class="faint small">{report.skipped.toLocaleString()} photo{report.skipped === 1 ? " was" : "s were"} skipped: {mode === "exact" ? "the drive isn't connected" : "no thumbnail yet"}.</p>
          {/if}
        </div>

        {#if groups.length}
          <div class="bulk small">
            <button class="link" onclick={tickSuggested}>Tick all but the suggested photo</button>
            <button class="link" onclick={() => (ticked = new Set())}>Untick all</button>
          </div>
          <div class="groups">
            {#each groups.slice(0, shown) as g, gi (g.members[0].photo.id)}
              <section class="group">
                <div class="ghead small">
                  <span class="faint">Group {gi + 1} · {g.members.length} photos</span>
                  <button class="link" onclick={() => keepAll(g)}>Keep all</button>
                </div>
                <div class="members">
                  {#each g.members as m (m.photo.id)}
                    {@const on = ticked.has(m.photo.id)}
                    <div class="m" class:on class:offline={!m.online}>
                      <button class="thumb" onclick={() => toggle(g, m)} disabled={!m.online} title={m.online ? (on ? "Ticked: goes to the Trash" : "Tick to move to the Trash") : "Drive not connected"}>
                        <img src={thumbUrl(m.photo)} alt="" loading="lazy" />
                        {#if on}<span class="badge out">To Trash</span>{:else if m.keep}<span class="badge keepword">Keep</span>{/if}
                        {#if !m.online}<span class="badge off"><CloudOff size={11} /></span>{/if}
                      </button>
                      <label class="tick small">
                        <input type="checkbox" checked={on} disabled={!m.online} onchange={() => toggle(g, m)} />
                        <span>Remove</span>
                      </label>
                      <div class="name" title={m.photo.path}>{m.photo.filename}</div>
                      <div class="meta faint" title={m.photo.folder}>{shortFolder(m.photo.folder)}</div>
                      <div class="meta faint">{[dims(m), formatBytes(m.photo.size)].filter(Boolean).join(" · ")}</div>
                      {#if m.photo.taken_at}<div class="meta faint">{formatDate(m.photo.taken_at)}</div>{/if}
                      <div class="row">
                        {#if m.photo.rating}<Stars value={m.photo.rating} size={10} />{/if}
                        {#if mode === "similar" && m.distance}<span class="meta faint" title="How different the fingerprints are, out of 64">≈{m.distance}</span>{/if}
                      </div>
                      <div class="row small">
                        <button class="link" onclick={() => keepOnly(g, m)}>Keep only this</button>
                        <button class="link" disabled={!m.online} onclick={() => api.reveal(m.photo.id).catch(errorToast)} title={`Show in ${fileManagerName}`}>Show</button>
                      </div>
                    </div>
                  {/each}
                </div>
              </section>
            {/each}
            {#if groups.length > shown}
              <button class="more" onclick={() => (shown += PAGE)}>Show {Math.min(PAGE, groups.length - shown)} more groups ({(groups.length - shown).toLocaleString()} left)</button>
            {/if}
          </div>
        {/if}
      {/if}

      <div class="buttons">
        <span class="faint small grow">{tickedCount ? `${tickedCount.toLocaleString()} ticked (${formatBytes(tickedBytes)})` : groups.length ? "Tick the photos you don't want." : ""}</span>
        <button onclick={close} disabled={busy}>Close</button>
        {#if groups.length}
          <button class="danger" onclick={trash} disabled={busy || running || !tickedCount}>
            {busy ? "Moving…" : `Move ${tickedCount.toLocaleString()} to the Trash`}
          </button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 1600; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .win { width: 920px; max-width: calc(100vw - 40px); max-height: min(900px, calc(100vh - 60px)); background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 16px 18px; box-shadow: var(--shadow); line-height: 1.45; display: flex; flex-direction: column; }
  .win.tall { height: calc(100vh - 60px); }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
  h3 { margin: 0; font-size: 15px; }
  .where { font-weight: 400; font-size: 12.5px; margin-left: 4px; }
  p { margin: 6px 0; }
  .small { font-size: 11.5px; }
  .options { display: flex; gap: 8px; align-items: center; }
  .options select { width: auto; }
  .grow { flex: 1; }
  .options .primary { display: flex; gap: 6px; align-items: center; }
  .progress { display: flex; flex-direction: column; gap: 4px; margin: 8px 0; }
  .bar { height: 4px; border-radius: 2px; background: var(--bg-4); overflow: hidden; }
  .bar > div { height: 100%; background: var(--accent); transition: width .2s; }
  .err { color: var(--danger); }
  .summary :global(svg) { vertical-align: -2px; margin-right: 4px; color: var(--success, #46a758); }
  .bulk { display: flex; gap: 14px; margin: 2px 0 6px; }
  .link { background: none !important; border: none; padding: 0; color: var(--accent); cursor: pointer; font-size: inherit; }
  .link:hover { text-decoration: underline; }
  .link:disabled { color: var(--text-3); text-decoration: none; }
  .groups { flex: 1; min-height: 0; overflow-y: auto; border: 1px solid var(--line-2); border-radius: 8px; background: var(--bg); padding: 8px; }
  .group { border-bottom: 1px solid var(--line); padding: 6px 2px 10px; }
  .group:last-of-type { border-bottom: none; }
  .ghead { display: flex; gap: 12px; align-items: center; margin-bottom: 6px; }
  .members { display: flex; gap: 10px; flex-wrap: wrap; }
  .m { width: 158px; display: flex; flex-direction: column; gap: 1px; font-size: 11.5px; }
  .thumb { position: relative; width: 158px; height: 118px; padding: 0; border-radius: 6px; overflow: hidden; background: var(--bg-3); border: 2px solid transparent; display: flex; align-items: center; justify-content: center; }
  .thumb img { max-width: 100%; max-height: 100%; object-fit: contain; }
  .m.on .thumb { border-color: var(--danger); }
  .m.on .thumb img { opacity: .55; }
  .m.offline .thumb { opacity: .7; }
  .badge { position: absolute; top: 4px; left: 4px; font-size: 10.5px; font-weight: 600; padding: 1px 6px; border-radius: 4px; }
  .badge.out { background: var(--danger); color: #fff; }
  .keepword { background: var(--accent); color: #fff; padding: 1px 6px; border-radius: 4px; font-size: 10.5px; font-weight: 600; }
  .badge.off { left: auto; right: 4px; background: rgba(0,0,0,.6); color: #fff; display: flex; }
  .tick { display: flex; gap: 5px; align-items: center; margin-top: 3px; }
  .tick input { accent-color: var(--danger); margin: 0; }
  .name { font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .meta { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: 11px; direction: ltr; }
  .row { display: flex; gap: 10px; align-items: center; min-height: 14px; }
  .more { margin: 10px auto 4px; display: block; }
  .buttons { display: flex; align-items: center; justify-content: flex-end; gap: 8px; margin-top: 12px; }
</style>
