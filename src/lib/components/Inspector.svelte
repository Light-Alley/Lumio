<script lang="ts">
  import { MapPin, X, Save, Undo2, CloudOff, Map as MapIcon, AlertTriangle, Image as ImageIcon, ChartArea } from "@lucide/svelte";
  import Stars from "./Stars.svelte";
  import Histogram from "./Histogram.svelte";
  import { app, selection, rate, setLabel, applyEdit, rootOnline, saveSetting } from "../store.svelte";
  import { api, thumbUrl, formatDate, formatBytes, formatExposure, LABELS, type Photo, type Edit } from "../api";

  let photos = $state<Photo[]>([]);
  let form = $state({ title: "", caption: "", creator: "", copyright: "", keywords: [] as string[] });
  let original = $state({ title: "", caption: "", creator: "", copyright: "", keywords: [] as string[] });
  let mixed = $state({ title: false, caption: false, creator: false, copyright: false });
  let kwInput = $state("");
  /** Keywords already used in the library, most used first, for suggestions. */
  let known = $state<string[]>([]);
  let knownVersion = -1;
  function loadKnown() {
    if (knownVersion === app.folderVersion) return;
    knownVersion = app.folderVersion;
    api.keywordList().then((l) => (known = l.map(([k]) => k))).catch(() => {});
  }
  let kwFocus = $state(false);
  /** Replace what is being typed with a suggested keyword. */
  function pick(k: string) {
    const parts = kwInput.split(/[,;]/);
    parts[parts.length - 1] = k;
    kwInput = parts.join(",");
    addKeyword();
  }
  const suggestions = $derived.by(() => {
    const q = kwInput.split(/[,;]/).pop()!.trim().toLowerCase();
    if (!q) return [];
    const have = new Set(form.keywords.map((k) => k.toLowerCase()));
    return known.filter((k) => k.toLowerCase().startsWith(q) && !have.has(k.toLowerCase())).slice(0, 8);
  });
  let saving = $state(false);
  let gpsText = $state("");

  const ids = $derived([...selection]);
  /** What the top of the panel shows for one photo: the picture or its histogram (remembered). */
  const showHist = $derived(app.settings.inspector_preview === "histogram");
  const one = $derived(photos.length === 1 ? photos[0] : null);
  const anyOffline = $derived(photos.some((p) => !rootOnline(p.root_id)));

  // Reload details whenever the selection (or the library) changes.
  let loadSeq = 0;
  $effect(() => {
    const sel = ids;
    app.photos; // re-run after library refresh
    const seq = ++loadSeq;
    if (!sel.length) { photos = []; return; }
    api.getPhotos(sel.slice(0, 2000)).then((ps) => {
      if (seq !== loadSeq) return;
      const had = new Set(photos.map((q) => q.id));
      const sameSet = ps.length === photos.length && ps.every((p) => had.has(p.id));
      photos = ps;
      if (!sameSet || !dirty) resetForm();
    });
  });

  function common<K extends keyof Photo>(k: K): { value: string; mixed: boolean } {
    const vals = new Set(photos.map((p) => (p[k] ?? "") as string));
    return vals.size === 1 ? { value: [...vals][0], mixed: false } : { value: "", mixed: true };
  }

  function resetForm() {
    const t = common("title"), c = common("caption"), cr = common("creator"), co = common("copyright");
    // Keywords: show those shared by all selected photos.
    let kws: string[] = photos.length ? [...photos[0].keywords] : [];
    for (const p of photos.slice(1)) kws = kws.filter((k) => p.keywords.some((x) => x.toLowerCase() === k.toLowerCase()));
    form = { title: t.value, caption: c.value, creator: cr.value, copyright: co.value, keywords: kws };
    original = { ...form, keywords: [...kws] };
    mixed = { title: t.mixed, caption: c.mixed, creator: cr.mixed, copyright: co.mixed };
    kwInput = "";
    gpsText = one && one.lat != null ? `${one.lat.toFixed(6)}, ${one.lon!.toFixed(6)}` : "";
  }

  const dirty = $derived(
    form.title !== original.title || form.caption !== original.caption || form.creator !== original.creator ||
    form.copyright !== original.copyright || form.keywords.join("|") !== original.keywords.join("|"),
  );

  function addKeyword() {
    const parts = kwInput.split(/[,;]/).map((s) => s.trim()).filter(Boolean);
    for (const k of parts) if (!form.keywords.some((x) => x.toLowerCase() === k.toLowerCase())) form.keywords.push(k);
    kwInput = "";
  }

  async function save() {
    if (kwInput.trim()) addKeyword();
    const edit: Edit = {};
    if (form.title !== original.title) edit.title = form.title;
    if (form.caption !== original.caption) edit.caption = form.caption;
    if (form.creator !== original.creator) edit.creator = form.creator;
    if (form.copyright !== original.copyright) edit.copyright = form.copyright;
    if (form.keywords.join("|") !== original.keywords.join("|")) {
      if (photos.length === 1) edit.keywords = form.keywords;
      else {
        const lower = (a: string[]) => a.map((x) => x.toLowerCase());
        edit.add_keywords = form.keywords.filter((k) => !lower(original.keywords).includes(k.toLowerCase()));
        edit.remove_keywords = original.keywords.filter((k) => !lower(form.keywords).includes(k.toLowerCase()));
      }
    }
    saving = true;
    await applyEdit(ids, edit);
    saving = false;
    original = { ...form, keywords: [...form.keywords] };
  }

  async function saveGps() {
    const m = gpsText.match(/(-?\d+(?:\.\d+)?)\s*[, ]\s*(-?\d+(?:\.\d+)?)/);
    if (!m) return;
    const lat = Number(m[1]), lon = Number(m[2]);
    if (Math.abs(lat) > 90 || Math.abs(lon) > 180) return;
    await applyEdit(ids, { gps: [lat, lon] });
  }

  const ratingCommon = $derived(photos.length && photos.every((p) => p.rating === photos[0].rating) ? photos[0].rating : 0);
  const labelCommon = $derived(photos.length && photos.every((p) => p.label === photos[0].label) ? photos[0].label : null);
  const withGps = $derived(photos.filter((p) => p.lat != null).length);
</script>

<div class="inspector">
  {#if !photos.length}
    <div class="none">
      <p class="muted">Select a photo to see and edit its details.</p>
      <div class="tips">
        <div><span class="kbd">Space</span> open viewer</div>
        <div><span class="kbd">0</span>–<span class="kbd">5</span> rate</div>
        <div><span class="kbd">6</span>–<span class="kbd">9</span> colour label</div>
        <div>Drag photos onto a folder to move them; hold <span class="kbd">Alt/⌥</span> to copy</div>
        <div>Right-click for "Open with", move, copy and more</div>
      </div>
    </div>
  {:else}
    <div class="head">
      {#if one}
        <div class="mode" role="group" aria-label="Show photo or histogram">
          <button class:on={!showHist} onclick={() => saveSetting("inspector_preview", "photo")} title="Show the photo"><ImageIcon size={13} /> Photo</button>
          <button class:on={showHist} onclick={() => saveSetting("inspector_preview", "histogram")} title="Show the histogram"><ChartArea size={13} /> Histogram</button>
        </div>
        <div class="preview">
          {#if showHist}
            <Histogram id={one.id} version={`${one.mtime}-${one.thumb}`} />
          {:else if one.thumb}<img src={thumbUrl(one)} alt="" />{/if}
        </div>
        <div class="fname" title={one.path}>{one.filename}</div>
        <div class="faint small ell" title={one.folder}>{one.folder}</div>
      {:else}
        <div class="stack">
          {#each photos.slice(0, 4) as p, i}
            {#if p.thumb}<img src={thumbUrl(p)} alt="" style:transform={`rotate(${(i - 1.5) * 5}deg)`} />{/if}
          {/each}
        </div>
        <div class="fname">{photos.length.toLocaleString()} photos selected</div>
        <div class="faint small">Changes apply to all of them</div>
      {/if}
      {#if anyOffline}<div class="warn"><CloudOff size={13} /> Drive not connected. Metadata can't be saved until it's back.</div>{/if}
      {#if !app.exiftool}<div class="warn"><AlertTriangle size={13} /> ExifTool not found. Metadata can't be written. See Settings.</div>{/if}
    </div>

    <div class="group">
      <div class="row">
        <Stars value={ratingCommon} size={17} onchange={(r) => rate(ids, r)} />
        <div class="labels">
          {#each LABELS as l}
            <button class="lab" class:on={labelCommon === l.name} style:background={l.color} title={l.name} aria-label={l.name} onclick={() => setLabel(ids, labelCommon === l.name ? "" : l.name)}></button>
          {/each}
        </div>
      </div>
    </div>

    <div class="group">
      <label class="field"><span>Title</span>
        <input type="text" bind:value={form.title} placeholder={mixed.title && form.title === original.title ? "Multiple values" : ""} />
      </label>
      <label class="field"><span>Caption / description</span>
        <textarea rows="3" bind:value={form.caption} placeholder={mixed.caption && form.caption === original.caption ? "Multiple values" : ""}></textarea>
      </label>
      <div class="field">
        <span class="lbl">Keywords{#if photos.length > 1} <span class="faint">(shared by all)</span>{/if}</span>
        <div class="chips">
          {#each form.keywords as k, i (k)}
            <span class="chip">{k}<button class="x" onclick={() => form.keywords.splice(i, 1)} aria-label={`Remove ${k}`}><X size={11} /></button></span>
          {/each}
          <input
            class="kw"
            type="text"
            placeholder="Add keyword…"
            title="Separate keywords with commas. Type Places > France > Paris for a keyword inside others, as in Lightroom."
            onfocus={() => { loadKnown(); kwFocus = true; }}
            bind:value={kwInput}
            onkeydown={(e) => { if (e.key === "Tab" && suggestions.length && !e.shiftKey) { e.preventDefault(); pick(suggestions[0]); } else if (e.key === "Enter" || e.key === ",") { e.preventDefault(); addKeyword(); } else if (e.key === "Backspace" && !kwInput && form.keywords.length) form.keywords.pop(); }}
            onblur={() => { kwFocus = false; if (kwInput.trim()) addKeyword(); }}
          />
        </div>
        {#if kwFocus && suggestions.length}
          <div class="suggest" role="listbox" aria-label="Keyword suggestions">
            {#each suggestions as k (k)}
              <button type="button" role="option" aria-selected="false" onmousedown={(e) => { e.preventDefault(); pick(k); }}>{k}</button>
            {/each}
          </div>
        {/if}
      </div>
      <label class="field"><span>Creator</span>
        <input type="text" bind:value={form.creator} placeholder={mixed.creator && form.creator === original.creator ? "Multiple values" : ""} />
      </label>
      <label class="field"><span>Copyright</span>
        <input type="text" bind:value={form.copyright} placeholder={mixed.copyright && form.copyright === original.copyright ? "Multiple values" : ""} />
      </label>
      <div class="actions">
        <button class="primary" disabled={!dirty || saving} onclick={save}><Save size={14} /> {saving ? "Saving…" : "Save to file"}</button>
        <button disabled={!dirty || saving} onclick={resetForm}><Undo2 size={14} /> Revert</button>
      </div>
    </div>

    <div class="group">
      <div class="gtitle"><MapPin size={13} /> Location</div>
      {#if one}
        <div class="gps">
          <input type="text" bind:value={gpsText} placeholder="Latitude, longitude" onkeydown={(e) => e.key === "Enter" && saveGps()} />
          <button onclick={saveGps} title="Save location">Set</button>
        </div>
        {#if one.place}<div class="small muted">{one.place}</div>{/if}
      {:else}
        <div class="small muted">{withGps} of {photos.length} have a location.</div>
      {/if}
      <div class="actions">
        <button onclick={() => (app.view = "map")}><MapIcon size={14} /> Place on map</button>
        {#if withGps}<button class="ghost" onclick={() => applyEdit(ids, { clear_gps: true })}>Remove location</button>{/if}
      </div>
    </div>

    {#if one}
      <div class="group exif">
        <div class="gtitle">Photo details</div>
        <dl>
          {#if one.taken_at}<dt>Taken</dt><dd>{formatDate(one.taken_at)}</dd>{/if}
          {#if one.make || one.model}<dt>Camera</dt><dd>{[one.make, one.model].filter(Boolean).join(" ")}</dd>{/if}
          {#if one.lens}<dt>Lens</dt><dd>{one.lens}</dd>{/if}
          {#if one.fnumber || one.exposure || one.iso}
            <dt>Exposure</dt>
            <dd>{[one.fnumber ? `f/${one.fnumber}` : "", formatExposure(one.exposure), one.iso ? `ISO ${one.iso}` : ""].filter(Boolean).join("  ·  ")}</dd>
          {/if}
          {#if one.focal}<dt>Focal length</dt><dd>{Math.round(one.focal)} mm</dd>{/if}
          {#if one.width}<dt>Dimensions</dt><dd>{one.width} × {one.height} ({((one.width * (one.height ?? 0)) / 1e6).toFixed(1)} MP)</dd>{/if}
          <dt>File</dt><dd>{one.ext.toUpperCase()} · {formatBytes(one.size)}</dd>
          <dt>Modified</dt><dd>{new Date(one.mtime).toLocaleString()}</dd>
        </dl>
      </div>
    {/if}
  {/if}
</div>

<style>
  .inspector { height: 100%; overflow-y: auto; padding-bottom: 30px; }
  .none { padding: 24px 18px; }
  .tips { display: grid; gap: 8px; color: var(--text-2); margin-top: 16px; line-height: 1.5; }
  .head { padding: 14px 16px 10px; text-align: center; border-bottom: 1px solid var(--line); }
  .preview { height: 170px; display: flex; align-items: center; justify-content: center; margin-bottom: 10px; }
  .mode { display: inline-flex; margin-bottom: 8px; border: 1px solid var(--line-2); border-radius: 6px; overflow: hidden; }
  .mode button { border: none; border-radius: 0; background: transparent; padding: 3px 9px; font-size: 11.5px; gap: 4px; color: var(--text-2); }
  .mode button + button { border-left: 1px solid var(--line-2); }
  .mode button.on { background: var(--accent-soft); color: var(--text); }
  .preview img { max-width: 100%; max-height: 100%; border-radius: 4px; box-shadow: 0 2px 10px rgba(0,0,0,.3); }
  .stack { height: 130px; position: relative; display: flex; align-items: center; justify-content: center; margin-bottom: 10px; }
  .stack img { position: absolute; max-width: 120px; max-height: 110px; border: 3px solid var(--bg-2); border-radius: 3px; box-shadow: 0 2px 8px rgba(0,0,0,.35); }
  .fname { font-weight: 600; word-break: break-all; }
  .small { font-size: 11.5px; }
  .ell { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .warn { margin-top: 8px; display: flex; gap: 6px; align-items: center; justify-content: center; color: var(--star); font-size: 11.5px; }
  .group { padding: 12px 16px; border-bottom: 1px solid var(--line); }
  .gtitle { font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: var(--text-3); font-weight: 600; margin-bottom: 8px; display: flex; gap: 6px; align-items: center; }
  .row { display: flex; align-items: center; justify-content: space-between; }
  .labels { display: flex; gap: 6px; }
  .lab { width: 16px; height: 16px; padding: 0; border-radius: 50%; border: 2px solid transparent; opacity: .35; }
  .lab.on { opacity: 1; border-color: var(--text); }
  .lbl { display: block; color: var(--text-2); font-size: 11.5px; margin-bottom: 4px; }
  .field { display: block; margin-bottom: 10px; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; padding: 4px; border: 1px solid var(--line-2); border-radius: 6px; background: var(--bg); min-height: 32px; }
  .chip { display: inline-flex; align-items: center; gap: 2px; background: var(--accent-soft); color: var(--text); border-radius: 10px; padding: 2px 4px 2px 8px; font-size: 12px; }
  .chip .x { background: none; border: none; padding: 1px; color: var(--text-2); }
  .suggest { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .suggest button { font-size: 12px; padding: 2px 8px; border-radius: 10px; border: 1px dashed var(--line-2); background: transparent; color: var(--text-2); }
  .suggest button:hover { border-style: solid; color: var(--text); }
  .kw { border: none !important; box-shadow: none !important; flex: 1; min-width: 90px; padding: 2px 4px !important; background: transparent !important; }
  .actions { display: flex; gap: 8px; margin-top: 4px; flex-wrap: wrap; }
  .gps { display: flex; gap: 6px; margin-bottom: 6px; }
  dl { display: grid; grid-template-columns: 92px 1fr; gap: 5px 8px; margin: 0; font-size: 12px; }
  dt { color: var(--text-3); }
  dd { margin: 0; color: var(--text); word-break: break-word; user-select: text; -webkit-user-select: text; }
</style>
