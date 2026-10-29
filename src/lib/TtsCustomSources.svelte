<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import SettingsSection from "./SettingsSection.svelte";
  import { ttsConfig, ttsRuntime, saveTtsConfig, type TtsCustomProtocol, type TtsCustomSource, type TtsEngine } from "./ttsConfig.svelte";

  let { compact = false }: { compact?: boolean } = $props();
  type TtsVoiceInfo = { id: string; styles: string[]; roles: string[] };

  let editingId = $state<string | null>(ttsConfig.engine.startsWith("custom:") ? ttsConfig.engine : null);
  let voices = $state<TtsVoiceInfo[]>([]);
  let models = $state<string[]>([]);
  let busy = $state(false);
  let errorMessage = $state("");
  let successMessage = $state("");
  let previewAudio: HTMLAudioElement | null = null;
  let previewing = $state(false);

  function save() {
    saveTtsConfig();
  }

  function updateSource(source: TtsCustomSource, patch: Partial<TtsCustomSource>) {
    const updated = { ...source, ...patch };
    ttsConfig.customSources = ttsConfig.customSources.map((item) => item.id === source.id ? updated : item);
    save();
    return updated;
  }

  function beginEdit(source: TtsCustomSource) {
    editingId = source.id;
    voices = [];
    models = [];
    errorMessage = "";
    successMessage = "";
    stopPreview();
  }

  function activate(source: TtsCustomSource) {
    ttsConfig.engine = source.id as TtsEngine;
    editingId = source.id;
    save();
  }

  function addSource() {
    const source: TtsCustomSource = {
      id: "custom:" + crypto.randomUUID(),
      name: "New custom source",
      protocol: "openAi",
      baseUrl: "",
      model: "",
      rate: 1,
      format: "mp3",
      preset: "",
      voiceIds: [],
      narrationVoice: "",
      dialogueVoice: "",
    };
    ttsConfig.customSources = [...ttsConfig.customSources, source];
    ttsConfig.engine = source.id;
    save();
    beginEdit(source);
  }

  function changeProtocol(source: TtsCustomSource, protocol: TtsCustomProtocol) {
    delete ttsRuntime.customKeys[source.id];
    const updated = updateSource(source, {
      protocol,
      model: "",
      rate: 1,
      format: "mp3",
      preset: "",
      voiceIds: [],
      narrationVoice: "",
      dialogueVoice: "",
    });
    beginEdit(updated);
  }

  function deleteSource(source: TtsCustomSource) {
    if (!confirm("Delete custom TTS source \"" + source.name + "\"?")) return;
    stopPreview();
    ttsConfig.customSources = ttsConfig.customSources.filter((item) => item.id !== source.id);
    delete ttsRuntime.customKeys[source.id];
    if (ttsConfig.engine === source.id) ttsConfig.engine = "device";
    if (editingId === source.id) editingId = null;
    voices = [];
    models = [];
    save();
  }

  function voiceOptions(source: TtsCustomSource): string[] {
    const configured = source.protocol === "openAi" ? source.voiceIds : [];
    return [...new Set([...voices.map((voice) => voice.id), ...configured])];
  }

  async function refreshVoices(source: TtsCustomSource) {
    busy = true;
    errorMessage = "";
    successMessage = "";
    try {
      voices = await invoke<TtsVoiceInfo[]>("tts_custom_voices", {
        source,
        apiKey: ttsRuntime.customKeys[source.id] ?? "",
      });
      successMessage = voices.length ? "Voices loaded." : "The endpoint did not advertise voices. Manual voice IDs remain available for OpenAI-compatible sources.";
    } catch (error) {
      errorMessage = String(error);
    } finally {
      busy = false;
    }
  }

  async function refreshModels(source: TtsCustomSource) {
    if (source.protocol !== "openAi") return;
    busy = true;
    errorMessage = "";
    successMessage = "";
    try {
      models = await invoke<string[]>("tts_custom_models", {
        source,
        apiKey: ttsRuntime.customKeys[source.id] ?? "",
      });
      successMessage = models.length ? "Models loaded." : "The endpoint did not advertise a model list; enter the model name manually.";
    } catch (error) {
      errorMessage = String(error);
    } finally {
      busy = false;
    }
  }

  function stopPreview() {
    if (previewAudio) {
      previewAudio.onended = null;
      previewAudio.onerror = null;
      previewAudio.pause();
      previewAudio.removeAttribute("src");
      previewAudio.load();
      previewAudio = null;
    }
    previewing = false;
  }

  async function testAndPreview(source: TtsCustomSource) {
    stopPreview();
    busy = true;
    errorMessage = "";
    successMessage = "";
    try {
      const encoded = await invoke<string>("tts_synthesize", {
        engine: source.id,
        text: "こんにちは。これは音声テストです。",
        baseUrl: "",
        apiKey: "",
        voice: source.narrationVoice,
        rate: source.rate,
        pitch: 0,
        options: {
          volume: 0, style: "", styleDegree: 1, role: "",
          leadingMs: -1, trailingMs: -1, commaMs: -1, semicolonMs: -1,
          enumerationCommaMs: -1, sentenceBoundaryMs: -1,
          format: source.format, sampleRate: 24000, effectsProfile: "",
        },
        customSource: source,
        customKey: ttsRuntime.customKeys[source.id] ?? "",
      });
      const mime = source.format === "wav" ? "audio/wav" : "audio/mpeg";
      const audio = new Audio("data:" + mime + ";base64," + encoded);
      previewAudio = audio;
      previewing = true;
      audio.onended = () => {
        if (previewAudio === audio) {
          previewAudio = null;
          previewing = false;
        }
      };
      audio.onerror = () => {
        if (previewAudio === audio) {
          previewAudio = null;
          previewing = false;
          errorMessage = "The test audio was generated but could not be played.";
        }
      };
      await audio.play();
      successMessage = "The source synthesized and played the Japanese test phrase.";
    } catch (error) {
      stopPreview();
      errorMessage = String(error);
    } finally {
      busy = false;
    }
  }

  onDestroy(stopPreview);
</script>

<SettingsSection compact={compact} title="Custom online TTS sources">
  <p class={compact ? "mb-1 text-xs text-base-content/60" : "mb-2 text-xs text-base-content/60"}>
    Configure OpenAI-compatible or VITS-compatible speech servers. Settings are stored locally; API keys are kept only for this session and must be entered again after restarting Hoshi.
  </p>

  {#if ttsConfig.customSources.length === 0}
    <p class="py-3 text-sm text-base-content/60">No custom sources configured.</p>
  {/if}

  {#each ttsConfig.customSources as source (source.id)}
    <div class="mb-3 rounded-box border border-base-300 p-3">
      <div class="flex flex-wrap items-center gap-2">
        <div class="min-w-0 flex-1">
          <p class="truncate text-sm font-semibold">{source.name || "Unnamed source"}</p>
          <p class="truncate text-xs text-base-content/60">{source.protocol === "openAi" ? "OpenAI-compatible" : "VITS-compatible"} · {source.baseUrl || "URL not configured"}</p>
        </div>
        {#if ttsConfig.engine === source.id}
          <span class="badge badge-primary badge-sm">Selected</span>
        {:else}
          <button class="btn btn-outline btn-xs" onclick={() => activate(source)}>Use</button>
        {/if}
        <button class="btn btn-ghost btn-xs" onclick={() => beginEdit(source)}>{editingId === source.id ? "Editing" : "Edit"}</button>
      </div>

      {#if editingId === source.id}
        <div class="mt-4 flex flex-col gap-3">
          <label class="flex flex-col gap-1 text-xs">
            <span>Source name</span>
            <input class="input input-sm w-full" value={source.name} onchange={(event) => updateSource(source, { name: event.currentTarget.value })} />
          </label>

          <label class="flex flex-col gap-1 text-xs">
            <span>Protocol</span>
            <select class="select select-sm w-full" value={source.protocol} onchange={(event) => changeProtocol(source, event.currentTarget.value as TtsCustomProtocol)}>
              <option value="openAi">OpenAI-compatible</option>
              <option value="vits">VITS-compatible</option>
            </select>
          </label>

          <label class="flex flex-col gap-1 text-xs">
            <span>Base URL</span>
            <input class="input input-sm w-full" type="url" autocomplete="off" value={source.baseUrl} placeholder="https://tts.example.com/v1/ or http://127.0.0.1:8080/" onchange={(event) => updateSource(source, { baseUrl: event.currentTarget.value })} />
          </label>

          <label class="flex flex-col gap-1 text-xs">
            <span>API key (session only)</span>
            <input class="input input-sm w-full" type="password" autocomplete="off" value={ttsRuntime.customKeys[source.id] ?? ""} placeholder="Optional API key" oninput={(event) => { ttsRuntime.customKeys[source.id] = event.currentTarget.value; }} />
          </label>

          {#if source.protocol === "openAi"}
            <div class="flex flex-col gap-1 text-xs">
              <span>Model</span>
              <div class="flex items-center gap-2">
                <input class="input input-sm min-w-0 flex-1" value={source.model} list={"tts-models-" + source.id} placeholder="Model name" onchange={(event) => updateSource(source, { model: event.currentTarget.value })} />
                <button class="btn btn-outline btn-sm shrink-0" onclick={() => refreshModels(source)} disabled={busy}>{busy ? "Working…" : "Refresh models"}</button>
              </div>
              <datalist id={"tts-models-" + source.id}>
                {#each models as model (model)}<option value={model}></option>{/each}
              </datalist>
              <label class="mt-2 flex flex-col gap-1">
                <span>Additional voice IDs (one per line; optional if the server has a voice-list endpoint)</span>
                <textarea class="textarea textarea-sm w-full" rows="2" value={source.voiceIds.join("\n")} onchange={(event) => updateSource(source, { voiceIds: event.currentTarget.value.split(/\r?\n/).map((value) => value.trim()).filter(Boolean) })}></textarea>
              </label>
            </div>
          {:else}
            <label class="flex flex-col gap-1 text-xs">
              <span>GPT-SoVITS preset (optional)</span>
              <input class="input input-sm w-full" value={source.preset} placeholder="Preset" onchange={(event) => updateSource(source, { preset: event.currentTarget.value })} />
            </label>
          {/if}

          <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label class="flex flex-col gap-1 text-xs">
              <span>Narration voice</span>
              <select class="select select-sm w-full" value={source.narrationVoice} onchange={(event) => updateSource(source, { narrationVoice: event.currentTarget.value })}>
                <option value="">Automatic / default voice</option>
                {#each voiceOptions(source) as voice (voice)}<option value={voice}>{voice}</option>{/each}
              </select>
            </label>
            <label class="flex flex-col gap-1 text-xs">
              <span>Dialogue voice (optional)</span>
              <select class="select select-sm w-full" value={source.dialogueVoice} onchange={(event) => updateSource(source, { dialogueVoice: event.currentTarget.value })}>
                <option value="">Same as narration</option>
                {#each voiceOptions(source) as voice (voice)}<option value={voice}>{voice}</option>{/each}
              </select>
            </label>
          </div>

          <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label class="flex flex-col gap-1 text-xs">
              <span>Audio format</span>
              <select class="select select-sm w-full" value={source.format} onchange={(event) => updateSource(source, { format: event.currentTarget.value as "mp3" | "wav" })}>
                <option value="mp3">MP3</option>
                <option value="wav">WAV</option>
              </select>
            </label>
            <label class="flex flex-col gap-1 text-xs">
              <span>Speech rate: {Number(source.rate).toFixed(2)}x</span>
              <input class="range range-xs w-full" type="range" min={source.protocol === "openAi" ? 0.25 : 0.5} max={source.protocol === "openAi" ? 4 : 3} step="0.05" value={source.rate} oninput={(event) => updateSource(source, { rate: Number(event.currentTarget.value) })} />
            </label>
          </div>

          <div class="flex flex-wrap items-center gap-2">
            <button class="btn btn-outline btn-sm" onclick={() => refreshVoices(source)} disabled={busy}>{busy ? "Working…" : "Refresh voices"}</button>
            <button class="btn btn-primary btn-sm" onclick={() => testAndPreview(source)} disabled={busy}>{busy ? "Testing…" : "Test & preview"}</button>
            {#if previewing}
              <button class="btn btn-ghost btn-sm" onclick={stopPreview}>Stop preview</button>
            {/if}
            <button class="btn btn-ghost btn-sm text-error" onclick={() => deleteSource(source)} disabled={busy}>Delete source</button>
          </div>
          {#if errorMessage}<p class="text-xs text-error" role="alert">{errorMessage}</p>{/if}
          {#if successMessage}<p class="text-xs text-success">{successMessage}</p>{/if}
        </div>
      {/if}
    </div>
  {/each}

  <button class="btn btn-outline btn-sm" onclick={addSource}>Add custom source</button>
</SettingsSection>
