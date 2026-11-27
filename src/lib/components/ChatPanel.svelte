<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { Sparkles, Send, RotateCcw, Images, Settings } from "@lucide/svelte";
  import { app, setScope } from "../store.svelte";
  import { api, errText, type AiSettings } from "../api";

  interface Msg { role: "user" | "assistant" | "error"; text: string; ids?: number[] | null; title?: string | null }
  let messages = $state<Msg[]>([]);
  let input = $state("");
  let busy = $state(false);
  let status = $state("");
  let settings = $state<AiSettings | null>(null);
  let list: HTMLDivElement | undefined = $state();

  const examples = [
    "Show my best photos from last summer",
    "Sunsets by the sea",
    "Photos taken in Paris with the 50mm lens",
    "Which of my photos have no keywords yet?",
  ];

  const ready = $derived(settings ? (settings.provider === "openai" ? true : settings.has_anthropic_key) : false);

  onMount(() => {
    api.aiSettings().then((s) => (settings = s));
    const un = listen<string>("ai-status", (e) => (status = e.payload));
    return () => { un.then((f) => f()); };
  });
  $effect(() => {
    if (!app.showSettings) api.aiSettings().then((s) => (settings = s));
  });

  async function send(text = input) {
    text = text.trim();
    if (!text || busy) return;
    input = "";
    messages.push({ role: "user", text });
    busy = true;
    await scroll();
    try {
      const r = await api.aiChat(text);
      messages.push({ role: "assistant", text: r.text || (r.result_ids ? "Here's what I found." : ""), ids: r.result_ids, title: r.result_title });
      if (r.result_ids) show(r.result_ids, r.result_title ?? text);
    } catch (e) {
      messages.push({ role: "error", text: errText(e) });
    } finally {
      busy = false;
      status = "";
      await scroll();
    }
  }
  function show(ids: number[], title: string) {
    setScope({ kind: "ai", ids, title: title || "AI results" });
    if (app.view === "map") app.view = "grid";
  }
  async function scroll() {
    await tick();
    list?.scrollTo({ top: list.scrollHeight, behavior: "smooth" });
  }
  function reset() {
    messages = [];
    api.aiReset();
  }
</script>

<div class="chat">
  <div class="head">
    <Sparkles size={15} /> <strong>Ask AI</strong>
    <span class="faint small">{settings ? (settings.provider === "openai" ? settings.openai_model : settings.anthropic_model) : ""}</span>
    <button class="ghost icon" style="margin-left:auto" title="New conversation" onclick={reset}><RotateCcw size={14} /></button>
  </div>

  <div class="list" bind:this={list}>
    {#if !messages.length}
      <div class="intro">
        <p>Describe the photos you're looking for in your own words. The assistant searches your catalogue's dates, places, keywords, ratings and cameras, and can glance at thumbnails for visual questions.</p>
        {#if settings && !ready}
          <div class="setup">
            <p>To use Claude, add an Anthropic API key. Or point Frameloft at a local model such as Ollama.</p>
            <button class="primary" onclick={() => (app.showSettings = true)}><Settings size={14} /> Open AI settings</button>
          </div>
        {:else}
          {#each examples as ex}<button class="example" onclick={() => send(ex)}>{ex}</button>{/each}
        {/if}
      </div>
    {/if}
    {#each messages as m}
      <div class="msg {m.role}">
        <div class="bubble">{m.text}</div>
        {#if m.ids?.length}
          <button class="results" onclick={() => show(m.ids!, m.title ?? "AI results")}><Images size={14} /> Show {m.ids.length} photo{m.ids.length === 1 ? "" : "s"}</button>
        {/if}
      </div>
    {/each}
    {#if busy}<div class="msg assistant"><div class="bubble typing">{status || "Thinking…"}</div></div>{/if}
  </div>

  <form class="composer" onsubmit={(e) => { e.preventDefault(); send(); }}>
    <textarea
      rows="2"
      placeholder="e.g. birthday photos from 2022 rated 3 stars or more"
      bind:value={input}
      onkeydown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); } }}
      disabled={busy}
    ></textarea>
    <button class="primary icon" type="submit" disabled={busy || !input.trim()} aria-label="Send"><Send size={15} /></button>
  </form>
</div>

<style>
  .chat { display: flex; flex-direction: column; height: 100%; }
  .head { display: flex; align-items: center; gap: 6px; padding: 10px 12px; border-bottom: 1px solid var(--line); }
  .head :global(svg) { color: var(--accent); }
  .small { font-size: 11px; }
  .list { flex: 1; overflow-y: auto; padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  .intro { color: var(--text-2); line-height: 1.5; display: flex; flex-direction: column; gap: 6px; }
  .intro p { margin: 0 0 6px; }
  .example { justify-content: flex-start; text-align: left; white-space: normal; background: var(--bg); }
  .setup { background: var(--bg-3); border-radius: 8px; padding: 10px; }
  .msg { display: flex; flex-direction: column; gap: 6px; max-width: 92%; }
  .msg.user { align-self: flex-end; align-items: flex-end; }
  .bubble { padding: 8px 11px; border-radius: 12px; line-height: 1.45; white-space: pre-wrap; user-select: text; -webkit-user-select: text; }
  .user .bubble { background: var(--accent); color: white; border-bottom-right-radius: 4px; }
  .assistant .bubble { background: var(--bg-3); border-bottom-left-radius: 4px; }
  .error .bubble { background: rgba(229,72,77,.15); color: var(--danger); }
  .typing { color: var(--text-2); font-style: italic; }
  .results { align-self: flex-start; }
  .composer { display: flex; gap: 6px; padding: 10px; border-top: 1px solid var(--line); align-items: flex-end; }
  .composer textarea { min-height: 40px; resize: none; }
  .composer button { height: 36px; width: 36px; justify-content: center; }
</style>
