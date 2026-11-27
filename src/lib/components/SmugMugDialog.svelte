<script lang="ts">
  // Upload the selected photos to a SmugMug gallery. The first time through it walks the
  // user through getting a SmugMug API key and signing in (see docs/smugmug.md).
  import { tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { X, ExternalLink, Check, LoaderCircle } from "@lucide/svelte";
  import { api, basename, errText } from "../api";
  import { toast } from "../store.svelte";
  import {
    publish, smugmug, remember, keep,
    type SmugStatus, type SmugAlbum, type SmugReport, type SmugProgress,
  } from "../publish.svelte";

  type Step = "loading" | "key" | "signin" | "pick" | "uploading" | "done";
  let step = $state<Step>("loading");
  let status = $state<SmugStatus | null>(null);
  let error = $state("");
  let busy = $state(false);

  // Setting up
  let appKey = $state("");
  let appSecret = $state("");
  let authUrl = $state("");
  let code = $state("");

  // Choosing where
  let albums = $state<SmugAlbum[]>([]);
  let albumsLoading = $state(false);
  let filter = $state("");
  let mode = $state<"existing" | "new">("existing");
  let chosen = $state<string>(remember("smugmug.album", ""));
  let newName = $state("");
  let privacy = $state<string>(remember("smugmug.privacy", "Unlisted"));
  let size = $state<"original" | "web">(remember("smugmug.size", "original"));
  let longEdge = $state<number>(remember("smugmug.longEdge", 3000));

  // Uploading
  let progress = $state<SmugProgress | null>(null);
  let report = $state<SmugReport | null>(null);
  let albumName = $state("");

  const ids = $derived(publish.smugmugIds ?? []);
  const count = $derived(ids.length);
  const shown = $derived(
    filter.trim()
      ? albums.filter((a) => (a.name + " " + a.path).toLowerCase().includes(filter.trim().toLowerCase()))
      : albums,
  );
  const fraction = $derived.by(() => {
    if (!progress || !progress.total) return 0;
    const part = progress.size ? Math.min(progress.bytes / progress.size, 1) : 0;
    return Math.min((progress.done + part) / progress.total, 1);
  });

  let opened = false;
  $effect(() => {
    if (publish.smugmugIds && !opened) {
      opened = true;
      start();
    } else if (!publish.smugmugIds) {
      opened = false;
    }
  });

  async function start() {
    step = "loading";
    error = "";
    report = null;
    progress = null;
    code = "";
    authUrl = "";
    newName = ""; // a new selection gets its own suggested name
    try {
      status = await smugmug.status();
      next();
    } catch (e) {
      error = errText(e);
      step = "key";
    }
    // Suggest the folder name as a gallery name.
    try {
      const [p] = await api.getPhotos(ids.slice(0, 1));
      if (p) newName = basename(p.folder);
    } catch { /* not important */ }
  }

  function next() {
    if (!status?.has_app_key) step = "key";
    else if (!status.connected) step = "signin";
    else {
      step = "pick";
      loadAlbums();
    }
  }

  async function loadAlbums() {
    albumsLoading = true;
    error = "";
    try {
      albums = await smugmug.albums();
      if (!albums.length) mode = "new";
      else if (!albums.some((a) => a.uri === chosen)) chosen = albums[0].uri;
    } catch (e) {
      error = errText(e);
    } finally {
      albumsLoading = false;
    }
  }

  async function run<T>(f: () => Promise<T>): Promise<T | undefined> {
    busy = true;
    error = "";
    try {
      return await f();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function saveKey() {
    const s = await run(() => smugmug.setAppKey(appKey, appSecret));
    if (s) {
      status = s;
      appKey = appSecret = "";
      next();
    }
  }

  async function signIn() {
    const url = await run(() => smugmug.beginSignIn());
    if (url) {
      authUrl = url;
      await tick();
      document.getElementById("smug-code")?.focus();
    }
  }

  async function finish() {
    const s = await run(() => smugmug.finishSignIn(code));
    if (s) {
      status = s;
      toast(`Signed in to SmugMug${s.nickname ? ` as ${s.nickname}` : ""}.`, "success");
      next();
    }
  }

  async function signOut(forgetKey: boolean) {
    const s = await run(() => smugmug.signOut(forgetKey));
    if (s) {
      status = s;
      albums = [];
      next();
    }
  }

  async function upload() {
    let album: SmugAlbum | undefined;
    if (mode === "new") {
      album = await run(() => smugmug.createAlbum(newName, privacy));
      if (!album) return;
      keep("smugmug.privacy", privacy);
    } else {
      album = albums.find((a) => a.uri === chosen);
      if (!album) { error = "Please choose a gallery."; return; }
    }
    keep("smugmug.album", album.uri);
    keep("smugmug.size", size);
    keep("smugmug.longEdge", longEdge);
    albumName = album.name;
    step = "uploading";
    progress = { done: 0, total: count, current: "", bytes: 0, size: 0 };
    const un = await listen<SmugProgress>("smugmug-progress", (e) => (progress = e.payload));
    try {
      report = await smugmug.upload(ids, album.uri, album.web_uri, { size, long_edge: size === "web" ? longEdge : null });
      step = "done";
    } catch (e) {
      error = errText(e);
      step = "pick";
    } finally {
      un();
    }
  }

  function close() {
    if (step === "uploading") return; // Cancel first.
    publish.smugmugIds = null;
  }

  const cleanErr = (m: string) => m.replace(/^Error: /, "");
</script>

{#if publish.smugmugIds}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
    <div class="win" role="dialog" aria-modal="true" aria-label="Upload to SmugMug" tabindex="-1" onkeydown={(e) => e.key === "Escape" && (e.stopPropagation(), close())}>
      <header>
        <h3>Upload {count === 1 ? "1 photo" : `${count} photos`} to SmugMug</h3>
        {#if step !== "uploading"}<button class="ghost icon" onclick={close} aria-label="Close"><X size={16} /></button>{/if}
      </header>

      {#if step === "loading"}
        <p class="muted"><LoaderCircle size={14} class="spin" /> Checking your SmugMug connection…</p>

      {:else if step === "key"}
        <p>SmugMug asks every app that uploads photos to use its own free "API key". You only do this once, and it takes about two minutes.</p>
        <ol>
          <li>Open <button class="link" onclick={() => smugmug.open(status?.apply_url ?? "https://api.smugmug.com/api/developer/apply")}>SmugMug's API key page <ExternalLink size={12} /></button> and sign in to SmugMug if asked.</li>
          <li>Fill in the short form. For example: name <em>Frameloft (personal)</em>, type <em>Application</em>, platform <em>Desktop</em>, use <em>Personal</em>, description <em>Uploading my own photos from Frameloft</em>. Accept the terms and submit.</li>
          <li>SmugMug then shows a <strong>Key</strong> and a <strong>Secret</strong>. Copy both into the boxes below.</li>
        </ol>
        <label class="field"><span>API key</span><input type="text" bind:value={appKey} spellcheck="false" autocomplete="off" /></label>
        <label class="field"><span>API secret</span><input type="password" bind:value={appSecret} spellcheck="false" autocomplete="off" /></label>
        <p class="faint small">Both are kept in your computer's keychain, never in the Frameloft catalogue.</p>
        <div class="buttons">
          <button onclick={close}>Cancel</button>
          <button class="primary" disabled={busy || !appKey.trim() || !appSecret.trim()} onclick={saveKey}>Save and continue</button>
        </div>

      {:else if step === "signin"}
        {#if !authUrl}
          <p>Next, let Frameloft upload to your SmugMug account. Your browser will open SmugMug; click <strong>Authorize</strong> there and SmugMug will show you a six-digit code.</p>
          <div class="buttons">
            <button class="ghost" onclick={() => signOut(true)}>Use a different API key</button>
            <span class="grow"></span>
            <button onclick={close}>Cancel</button>
            <button class="primary" disabled={busy} onclick={signIn}>Sign in to SmugMug</button>
          </div>
        {:else}
          <p>Type the six-digit code SmugMug is showing you.</p>
          <label class="field"><span>Code from SmugMug</span>
            <input id="smug-code" type="text" inputmode="numeric" maxlength="12" bind:value={code} onkeydown={(e) => e.key === "Enter" && code.trim() && finish()} />
          </label>
          <p class="faint small">Browser didn't open? <button class="link" onclick={() => smugmug.open(authUrl)}>Open SmugMug's page</button></p>
          <div class="buttons">
            <button onclick={() => (authUrl = "")}>Back</button>
            <button class="primary" disabled={busy || !code.trim()} onclick={finish}>Finish signing in</button>
          </div>
        {/if}

      {:else if step === "pick"}
        <p class="who"><Check size={14} /> Signed in{status?.nickname ? ` as ${status.nickname}` : ""}. <button class="link" onclick={() => signOut(false)}>Sign out</button></p>
        <div class="seg">
          <button class:active={mode === "existing"} disabled={!albums.length && !albumsLoading} onclick={() => (mode = "existing")}>Existing gallery</button>
          <button class:active={mode === "new"} onclick={() => (mode = "new")}>New gallery</button>
        </div>
        {#if mode === "existing"}
          {#if albumsLoading}
            <p class="muted"><LoaderCircle size={14} class="spin" /> Loading your galleries…</p>
          {:else}
            {#if albums.length > 8}<input type="search" placeholder="Find a gallery" bind:value={filter} />{/if}
            <div class="albums" role="listbox" aria-label="Galleries">
              {#each shown as a (a.uri)}
                <button role="option" aria-selected={chosen === a.uri} class:active={chosen === a.uri} onclick={() => (chosen = a.uri)} ondblclick={upload}>
                  <span class="name">{a.name}</span>
                  <span class="faint">{a.path || ""}{a.privacy ? ` · ${a.privacy}` : ""}{a.image_count != null ? ` · ${a.image_count}` : ""}</span>
                </button>
              {:else}
                <p class="faint">No galleries match.</p>
              {/each}
            </div>
          {/if}
        {:else}
          <label class="field"><span>Gallery name</span><input type="text" bind:value={newName} maxlength="120" /></label>
          <label class="field"><span>Who can see it</span>
            <select bind:value={privacy}>
              <option value="Unlisted">Unlisted: anyone with the link</option>
              <option value="Public">Public: anyone, and shown on your site</option>
              <option value="Private">Private: only you</option>
            </select>
          </label>
        {/if}
        <h4>What to send</h4>
        <label class="radio"><input type="radio" name="smsize" value="original" bind:group={size} />
          <span><strong>Original files.</strong> JPEG, PNG, HEIC and videos go as they are; RAW and TIFF files go as full-size JPEG copies.</span></label>
        <label class="radio"><input type="radio" name="smsize" value="web" bind:group={size} />
          <span><strong>Smaller JPEG copies</strong>, longest side
            <select class="inline-select" bind:value={longEdge} onclick={() => (size = "web")}>
              {#each [2048, 3000, 4000, 6000] as n}<option value={n}>{n} px</option>{/each}
            </select>. Quicker to upload. Videos still go as they are.</span></label>
        {#if error}<p class="err">{cleanErr(error)}</p>{/if}
        <div class="buttons">
          <button onclick={close}>Cancel</button>
          <button class="primary" disabled={busy || albumsLoading || (mode === "new" ? !newName.trim() : !chosen)} onclick={upload}>
            {busy ? "Creating gallery…" : `Upload ${count === 1 ? "photo" : `${count} photos`}`}
          </button>
        </div>

      {:else if step === "uploading" && progress}
        <p>Uploading to <strong>{albumName}</strong>: {Math.min(progress.done + 1, progress.total)} of {progress.total}</p>
        <div class="bar"><div style:width={`${(fraction * 100).toFixed(1)}%`}></div></div>
        <p class="faint small file">{progress.current}</p>
        <p class="faint small">Keep this window open until the upload finishes.</p>
        <div class="buttons"><button onclick={() => smugmug.cancel()}>Cancel upload</button></div>

      {:else if step === "done" && report}
        {#if report.done}
          <p class="who"><Check size={14} /> {report.done === 1 ? "1 photo" : `${report.done} photos`} uploaded to <strong>{albumName}</strong>.</p>
        {/if}
        {#if report.cancelled}<p class="muted">The upload was cancelled; photos already sent stay in the gallery.</p>{/if}
        {#if report.failed.length}
          <p class="err">{report.failed.length === 1 ? "1 photo" : `${report.failed.length} photos`} could not be uploaded:</p>
          <ul class="failed">{#each report.failed as [name, why]}<li><strong>{name}</strong>: {cleanErr(why)}</li>{/each}</ul>
        {/if}
        <div class="buttons">
          {#if report.web_uri && report.done}<button onclick={() => smugmug.open(report!.web_uri)}><ExternalLink size={14} /> Open gallery</button>{/if}
          <button class="primary" onclick={close}>Done</button>
        </div>
      {/if}

      {#if error && step !== "pick"}<p class="err">{cleanErr(error)}</p>{/if}
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 1600; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .win { width: 520px; max-width: calc(100vw - 40px); max-height: calc(100vh - 40px); overflow-y: auto; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; padding: 16px 18px; box-shadow: var(--shadow); line-height: 1.45; }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
  h3 { margin: 0; font-size: 15px; }
  h4 { font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: var(--text-3); margin: 14px 0 6px; }
  p { margin: 6px 0 10px; }
  ol { padding-left: 20px; margin: 6px 0 12px; } ol li { margin-bottom: 6px; }
  .small { font-size: 11.5px; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; align-items: center; }
  .grow { flex: 1; }
  .link { background: none; border: none; padding: 0; color: var(--accent); display: inline-flex; gap: 3px; cursor: pointer; }
  .link:hover:not(:disabled) { background: none; text-decoration: underline; }
  .who { color: var(--success); display: flex; align-items: center; gap: 6px; }
  .who .link { margin-left: 4px; }
  .seg { display: flex; gap: 6px; margin-bottom: 10px; }
  .albums { max-height: 220px; overflow-y: auto; border: 1px solid var(--line-2); border-radius: 8px; padding: 4px; margin-top: 6px; background: var(--bg); }
  .albums button { width: 100%; background: none; border: 1px solid transparent; justify-content: space-between; text-align: left; }
  .albums button.active { background: var(--accent-soft); border-color: var(--accent); }
  .albums .name { overflow: hidden; text-overflow: ellipsis; }
  .albums .faint { font-size: 11px; overflow: hidden; text-overflow: ellipsis; }
  .albums p { margin: 8px; }
  .radio { display: flex; gap: 8px; align-items: flex-start; margin-bottom: 8px; }
  .radio input { margin-top: 3px; accent-color: var(--accent); }
  .inline-select { width: auto; padding: 1px 4px; }
  .bar { height: 8px; border-radius: 4px; background: var(--bg-4); overflow: hidden; margin: 10px 0 4px; }
  .bar div { height: 100%; background: var(--accent); transition: width .25s; }
  .file { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .err { color: var(--danger); white-space: pre-wrap; }
  .failed { max-height: 140px; overflow-y: auto; padding-left: 18px; font-size: 12px; user-select: text; -webkit-user-select: text; }
  :global(.spin) { animation: spin 1s linear infinite; vertical-align: -2px; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
