<script lang="ts">
  import { X, Check, AlertTriangle } from "@lucide/svelte";
  import { app, saveSetting, toast, errorToast } from "../store.svelte";
  import { api, formatBytes, type AiSettings, type AppStatus, type OfflineStats } from "../api";

  let tab = $state<"general" | "ai" | "map" | "about">("general");
  let ai = $state<AiSettings | null>(null);
  let status = $state<AppStatus | null>(null);
  let offline = $state<OfflineStats | null>(null);
  let anthropicKey = $state("");
  let openaiKey = $state("");

  $effect(() => {
    if (app.showSettings) {
      api.aiSettings().then((s) => (ai = s));
      api.appStatus().then((s) => (status = s));
      loadOffline();
    }
  });

  function loadOffline() {
    api.offlinePreviewStats().then((s) => (offline = s)).catch(() => (offline = null));
  }
  function setOffline(key: string, value: string) {
    saveSetting(key, value);
    setTimeout(loadOffline, 300);
  }
  async function clearOffline() {
    try {
      await api.clearOfflinePreviews();
      toast("Saved previews deleted.", "success");
      loadOffline();
    } catch (e) { errorToast(e); }
  }

  function setAi(key: string, value: string) {
    api.setSetting(key, value).then(() => api.aiSettings().then((s) => (ai = s))).catch(errorToast);
  }
  async function saveKey(which: "anthropic" | "openai", key: string) {
    try {
      await api.aiSetKey(which, key);
      ai = await api.aiSettings();
      if (which === "anthropic") anthropicKey = ""; else openaiKey = "";
      toast(key ? "API key saved in your system keychain." : "API key removed.", "success");
    } catch (e) { errorToast(e); }
  }
  // Save the name a moment after typing stops; the plate updates as you type.
  let nameTimer: ReturnType<typeof setTimeout>;
  function setOwnerName(v: string) {
    app.settings.owner_name = v;
    clearTimeout(nameTimer);
    nameTimer = setTimeout(() => saveSetting("owner_name", v.trim()), 400);
  }
  function setTheme(t: "dark" | "light") {
    app.theme = t;
    saveSetting("theme", t);
  }
</script>

{#if app.showSettings}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && (app.showSettings = false)}>
    <div class="settings" role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1" onkeydown={(e) => e.key === "Escape" && (app.showSettings = false)}>
      <nav>
        <h2>Settings</h2>
        <button class:active={tab === "general"} onclick={() => (tab = "general")}>General</button>
        <button class:active={tab === "ai"} onclick={() => (tab = "ai")}>AI assistant</button>
        <button class:active={tab === "map"} onclick={() => (tab = "map")}>Map</button>
        <button class:active={tab === "about"} onclick={() => (tab = "about")}>About</button>
      </nav>
      <section>
        <button class="ghost icon close" onclick={() => (app.showSettings = false)} aria-label="Close"><X size={16} /></button>
        {#if tab === "general"}
          <h3>Identity plate</h3>
          <label class="field"><span>Your name, shown under "Lumio by Light and Alley" at the top left. Leave blank to hide it.</span>
            <input type="text" placeholder="e.g. Jane Smith Photography" maxlength="60" value={app.settings.owner_name ?? ""} oninput={(e) => setOwnerName((e.target as HTMLInputElement).value)} />
          </label>
          <h3>Appearance</h3>
          <div class="seg">
            <button class:active={app.theme === "dark"} onclick={() => setTheme("dark")}>Dark</button>
            <button class:active={app.theme === "light"} onclick={() => setTheme("light")}>Light</button>
          </div>
          <h3>Previews for offline viewing</h3>
          <p class="muted">Lumio keeps a sharp copy of each photo (as well as the small thumbnail), so the large view and filmstrip look crisp straight away, even with the drive unplugged or the network drive away.</p>
          <label class="radio"><input type="radio" name="offline" checked={(app.settings.offline_previews ?? "removable") === "removable"} onchange={() => setOffline("offline_previews", "removable")} />
            <span>For external, network and cloud drives <span class="muted">(recommended)</span></span></label>
          <label class="radio"><input type="radio" name="offline" checked={app.settings.offline_previews === "all"} onchange={() => setOffline("offline_previews", "all")} />
            <span>For every library folder, including this computer's own disk</span></label>
          <label class="radio"><input type="radio" name="offline" checked={app.settings.offline_previews === "off"} onchange={() => setOffline("offline_previews", "off")} />
            <span>Off <span class="muted">(only thumbnails are kept)</span></span></label>
          <label class="field"><span>Size</span>
            <select value={app.settings.offline_preview_size ?? "1600"} onchange={(e) => setOffline("offline_preview_size", (e.target as HTMLSelectElement).value)}>
              <option value="1600">Standard, 1600 px (about 0.3 MB a photo)</option>
              <option value="2560">Large, 2560 px (about 0.8 MB a photo)</option>
            </select>
          </label>
          {#if offline}
            <p class="muted">
              {offline.saved.toLocaleString()} saved, using {formatBytes(offline.bytes)}.
              {#if offline.waiting}{offline.waiting.toLocaleString()} still to make; Lumio does these in the background while the drive is connected.{/if}
              {#if offline.drives.length}<br />Kept for: {offline.drives.join(", ")}.{:else if (app.settings.offline_previews ?? "removable") !== "off"}<br />None of your connected library folders need them with this setting.{/if}
            </p>
            {#if offline.saved}<button onclick={clearOffline}>Delete saved previews</button>{/if}
          {/if}
          <h3>Sharing</h3>
          <p class="muted">What to send when you share photos by email, WhatsApp or a website. Your originals are never changed.</p>
          <label class="radio"><input type="radio" name="share" checked={(app.settings.share_size ?? "original") === "original"} onchange={() => saveSetting("share_size", "original")} />
            <span><strong>Full size</strong> (recommended). JPEG and PNG photos are sent as they are; RAW and HEIC photos are turned into high-quality JPEG copies first.</span></label>
          <label class="radio"><input type="radio" name="share" checked={app.settings.share_size === "small"} onchange={() => saveSetting("share_size", "small")} />
            <span><strong>Smaller copies</strong> (2048 pixels on the long side). Quicker to send, and they leave out the camera details and the location where the photo was taken.</span></label>
          <h3>RAW files</h3>
          <p class="muted">Where should ratings, keywords and other edits go for RAW photos (CR3, NEF, ARW, DNG…)?</p>
          <label class="radio"><input type="radio" name="raw" checked={(app.settings.raw_write_mode ?? "sidecar") === "sidecar"} onchange={() => saveSetting("raw_write_mode", "sidecar")} />
            <span><strong>XMP sidecar file</strong> (recommended). The RAW file is never modified. Lightroom, Bridge, Capture One and darktable read these.</span></label>
          <label class="radio"><input type="radio" name="raw" checked={app.settings.raw_write_mode === "embed"} onchange={() => saveSetting("raw_write_mode", "embed")} />
            <span><strong>Inside the RAW file.</strong> Keeps everything in one file but rewrites the original.</span></label>
          <h3>Metadata engine</h3>
          {#if status}
            {#if status.exiftool}
              <p class="ok"><Check size={14} /> ExifTool found: <code>{status.exiftool_path}</code></p>
            {:else}
              <p class="bad"><AlertTriangle size={14} /> ExifTool was not found. Lumio can browse photos but can't read or write most metadata. Install it from exiftool.org (on a Mac: <code>brew install exiftool</code>) and restart Lumio.</p>
            {/if}
          {/if}
        {:else if tab === "ai" && ai}
          <h3>Provider</h3>
          <label class="radio"><input type="radio" name="prov" checked={ai.provider === "anthropic"} onchange={() => setAi("ai_provider", "anthropic")} />
            <span><strong>Claude (Anthropic)</strong>. Most capable. Needs an API key from console.anthropic.com; usage is billed by Anthropic.</span></label>
          <label class="radio"><input type="radio" name="prov" checked={ai.provider === "openai"} onchange={() => setAi("ai_provider", "openai")} />
            <span><strong>Local or OpenAI-compatible model</strong>, e.g. Ollama or LM Studio running on this computer, so nothing leaves your machine.</span></label>

          {#if ai.provider === "anthropic"}
            <label class="field"><span>Anthropic API key {#if ai.has_anthropic_key}<em class="ok">saved</em>{/if}</span>
              <div class="inline">
                <input type="password" placeholder={ai.has_anthropic_key ? "••••••••••••  (enter a new key to replace)" : "sk-ant-…"} bind:value={anthropicKey} />
                <button class="primary" disabled={!anthropicKey.trim()} onclick={() => saveKey("anthropic", anthropicKey)}>Save</button>
                {#if ai.has_anthropic_key}<button onclick={() => saveKey("anthropic", "")}>Remove</button>{/if}
              </div>
            </label>
            <label class="field"><span>Model</span>
              <input type="text" value={ai.anthropic_model} onchange={(e) => setAi("ai_anthropic_model", (e.target as HTMLInputElement).value.trim())} />
            </label>
            <label class="field"><span>Effort (higher is more thorough but slower and costs more)</span>
              <select value={ai.effort} onchange={(e) => setAi("ai_effort", (e.target as HTMLSelectElement).value)}>
                <option value="low">Low (fast, good for most searches)</option>
                <option value="medium">Medium</option>
                <option value="high">High</option>
              </select>
            </label>
          {:else}
            <label class="field"><span>Server address</span>
              <input type="text" value={ai.openai_base_url} onchange={(e) => setAi("ai_openai_base_url", (e.target as HTMLInputElement).value.trim())} />
            </label>
            <label class="field"><span>Model name (must support tool calling)</span>
              <input type="text" value={ai.openai_model} onchange={(e) => setAi("ai_openai_model", (e.target as HTMLInputElement).value.trim())} />
            </label>
            <label class="field"><span>API key (optional; not needed for Ollama) {#if ai.has_openai_key}<em class="ok">saved</em>{/if}</span>
              <div class="inline">
                <input type="password" bind:value={openaiKey} />
                <button class="primary" disabled={!openaiKey.trim()} onclick={() => saveKey("openai", openaiKey)}>Save</button>
                {#if ai.has_openai_key}<button onclick={() => saveKey("openai", "")}>Remove</button>{/if}
              </div>
            </label>
          {/if}
          <h3>Privacy</h3>
          <label class="check"><input type="checkbox" checked={ai.allow_images} onchange={(e) => setAi("ai_allow_images", (e.target as HTMLInputElement).checked ? "1" : "0")} />
            <span>Let the assistant look at small thumbnails when metadata isn't enough (e.g. "photos of dogs"). Off means it only sees file names, dates, places, keywords and camera details.</span></label>
        {:else if tab === "map"}
          <h3>Map style</h3>
          {#each [["osm", "OpenStreetMap"], ["carto", "CARTO Voyager"], ["cartodark", "CARTO Dark"], ["satellite", "Satellite (Esri)"], ["topo", "Topographic (OpenTopoMap)"]] as [id, name]}
            <label class="radio"><input type="radio" name="tiles" checked={(app.settings.map_tiles ?? "osm") === id} onchange={() => saveSetting("map_tiles", id)} /><span>{name}</span></label>
          {/each}
          <p class="muted">Map tiles and place search need an internet connection. Place search uses OpenStreetMap Nominatim.</p>
        {:else if tab === "about" && status}
          <h3>Lumio {status.version} by Light and Alley</h3>
          <p class="muted">A lightweight, open-source photo browser and digital asset manager. Built with Rust and Tauri. MIT licence.</p>
          <dl>
            <dt>Photos in catalogue</dt><dd>{status.photo_count.toLocaleString()}</dd>
            <dt>Catalogue & thumbnails</dt><dd><code>{status.data_dir}</code></dd>
          </dl>
          <p class="muted">Metadata is read and written with ExifTool by Phil Harvey. Maps © OpenStreetMap contributors.</p>
        {/if}
      </section>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 1500; background: rgba(0,0,0,.45); display: flex; align-items: center; justify-content: center; }
  .settings { width: 760px; max-width: calc(100vw - 40px); height: 560px; max-height: calc(100vh - 40px); display: flex; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 12px; box-shadow: var(--shadow); overflow: hidden; }
  nav { width: 180px; background: var(--bg-3); padding: 14px 8px; display: flex; flex-direction: column; gap: 2px; }
  nav h2 { font-size: 14px; margin: 0 8px 12px; }
  nav button { background: none; border: none; justify-content: flex-start; }
  nav button.active { background: var(--accent-soft); }
  section { flex: 1; padding: 18px 22px; overflow-y: auto; position: relative; line-height: 1.45; }
  .close { position: absolute; top: 10px; right: 10px; }
  h3 { font-size: 12px; text-transform: uppercase; letter-spacing: .06em; color: var(--text-3); margin: 18px 0 8px; }
  h3:first-of-type { margin-top: 4px; }
  .radio, .check { display: flex; gap: 8px; align-items: flex-start; margin-bottom: 8px; }
  .radio input, .check input { margin-top: 3px; accent-color: var(--accent); }
  .inline { display: flex; gap: 6px; }
  .seg { display: flex; gap: 6px; }
  .ok { color: var(--success); display: flex; gap: 6px; align-items: center; font-style: normal; }
  em.ok { display: inline; margin-left: 6px; }
  .bad { color: var(--star); display: flex; gap: 6px; align-items: flex-start; }
  code { font-size: 11.5px; background: var(--bg); padding: 1px 4px; border-radius: 4px; word-break: break-all; user-select: text; -webkit-user-select: text; }
  dl { display: grid; grid-template-columns: 170px 1fr; gap: 6px; }
  dt { color: var(--text-3); } dd { margin: 0; }
</style>
