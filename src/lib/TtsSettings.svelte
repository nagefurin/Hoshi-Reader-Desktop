<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { ttsConfig, ttsRuntime, saveTtsConfig, type TtsEngine } from "./ttsConfig.svelte";
  import SettingsSection from "./SettingsSection.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingToggle from "./SettingToggle.svelte";
  import SettingStepper from "./SettingStepper.svelte";
  import { sasayakiConfig, saveSasayakiConfig } from "./sasayakiConfig.svelte";

  let { compact = false }: { compact?: boolean } = $props();

  let voices = $state<SpeechSynthesisVoice[]>([]);
  let edgeVoices = $state<string[]>([]);
  let edgeVoiceLoading = $state(false);
  let edgeVoiceError = $state("");
  let azureVoices = $state<string[]>([]);
  let azureVoiceLoading = $state(false);
  let azureVoiceError = $state("");
  let translatorVoices = $state<string[]>([]);
  let translatorVoiceLoading = $state(false);
  let translatorVoiceError = $state("");
  let googleCloudVoices = $state<string[]>([]);
  let googleCloudVoiceLoading = $state(false);
  let googleCloudVoiceError = $state("");

  function changeEngine(value: string) {
    const previous = ttsConfig.engine;
    const next = value as TtsEngine;
    if (next === "edge" && previous !== "edge") ttsConfig.pitch = 0;
    else if (["azure", "microsoftTranslator", "googleCloud"].includes(next) && previous !== next) ttsConfig.pitch = 0;
    else if (next === "device" && previous !== "device") ttsConfig.pitch = 1;
    ttsConfig.engine = next;
    saveTtsConfig();
  }

  async function refreshEdgeVoices() {
    edgeVoiceLoading = true;
    edgeVoiceError = "";
    try {
      edgeVoices = await invoke<string[]>("tts_edge_voices");
      if (edgeVoices.length && !edgeVoices.includes(ttsConfig.edgeVoice)) {
        ttsConfig.edgeVoice = edgeVoices.includes("ja-JP-NanamiNeural")
          ? "ja-JP-NanamiNeural"
          : edgeVoices[0];
        saveTtsConfig();
      }
      if (!edgeVoices.length) edgeVoiceError = "No Japanese Edge voices were returned.";
    } catch (error) {
      edgeVoiceError = String(error);
    } finally {
      edgeVoiceLoading = false;
    }
  }

  async function refreshAzureVoices() {
    if (!ttsRuntime.azureKey.trim()) {
      azureVoiceError = "Enter an Azure Speech subscription key before refreshing voices.";
      return;
    }
    azureVoiceLoading = true;
    azureVoiceError = "";
    try {
      azureVoices = await invoke<string[]>("tts_azure_voices", {
        region: ttsConfig.azureRegion,
        apiKey: ttsRuntime.azureKey,
      });
      if (azureVoices.length && !azureVoices.includes(ttsConfig.azureVoice)) {
        ttsConfig.azureVoice = azureVoices.includes("ja-JP-NanamiNeural") ? "ja-JP-NanamiNeural" : azureVoices[0];
        saveTtsConfig();
      }
      if (!azureVoices.length) azureVoiceError = "No Japanese Azure voices were returned.";
    } catch (error) {
      azureVoiceError = String(error);
    } finally {
      azureVoiceLoading = false;
    }
  }

  async function refreshTranslatorVoices() {
    translatorVoiceLoading = true;
    translatorVoiceError = "";
    try {
      translatorVoices = await invoke<string[]>("tts_microsoft_translator_voices");
      if (translatorVoices.length && !translatorVoices.includes(ttsConfig.microsoftTranslatorVoice)) {
        ttsConfig.microsoftTranslatorVoice = translatorVoices[0];
        saveTtsConfig();
      }
      if (!translatorVoices.length) translatorVoiceError = "No Japanese Microsoft Translator voices were returned.";
    } catch (error) {
      translatorVoiceError = String(error);
    } finally {
      translatorVoiceLoading = false;
    }
  }

  async function refreshGoogleCloudVoices() {
    if (!ttsRuntime.googleCloudKey.trim()) {
      googleCloudVoiceError = "Enter a Google Cloud Text-to-Speech API key before refreshing voices.";
      return;
    }
    googleCloudVoiceLoading = true;
    googleCloudVoiceError = "";
    try {
      googleCloudVoices = await invoke<string[]>("tts_google_cloud_voices", {
        apiKey: ttsRuntime.googleCloudKey,
      });
      if (googleCloudVoices.length && !googleCloudVoices.includes(ttsConfig.googleCloudVoice)) {
        ttsConfig.googleCloudVoice = googleCloudVoices[0];
        saveTtsConfig();
      }
      if (!googleCloudVoices.length) googleCloudVoiceError = "No Japanese Google Cloud voices were returned.";
    } catch (error) {
      googleCloudVoiceError = String(error);
    } finally {
      googleCloudVoiceLoading = false;
    }
  }

  $effect(() => {
    if (ttsConfig.engine === "edge") void refreshEdgeVoices();
    else if (ttsConfig.engine === "microsoftTranslator") void refreshTranslatorVoices();
    if ((ttsConfig.engine as string) === "vits") changeEngine("device");
  });

  $effect(() => {
    if (typeof window === "undefined" || !("speechSynthesis" in window)) return;
    const speech = window.speechSynthesis;
    const refresh = () => {
      voices = speech.getVoices().slice().sort((a, b) => {
        const aJapanese = a.lang.toLowerCase().startsWith("ja") ? 0 : 1;
        const bJapanese = b.lang.toLowerCase().startsWith("ja") ? 0 : 1;
        return aJapanese - bJapanese || a.name.localeCompare(b.name);
      });
    };
    refresh();
    speech.addEventListener("voiceschanged", refresh);
    return () => speech.removeEventListener("voiceschanged", refresh);
  });

  function save() {
    saveTtsConfig();
  }

  function addReplacement() {
    ttsConfig.replacements.push({ id: crypto.randomUUID(), pattern: "", replacement: "" });
    save();
  }

  function removeReplacement(id: string) {
    ttsConfig.replacements = ttsConfig.replacements.filter((rule) => rule.id !== id);
    save();
  }
</script>

<SettingsSection compact={compact} title="Speech Engine">
  <SettingRow compact={compact} label="Provider">
    <select class="select select-sm w-52" value={ttsConfig.engine} onchange={(event) => changeEngine(event.currentTarget.value)}>
      <option value="device">Device / system voice</option>
      <option value="edge">Microsoft Edge (online)</option>
      <option value="azure">Microsoft Azure Speech</option>
      <option value="microsoftTranslator">Microsoft Translator</option>
      <option value="googleCloud">Google Cloud Text-to-Speech</option>
      <option value="googleTranslate">Google Translate (online)</option>
      <option value="openAi">OpenAI-compatible / custom endpoint</option>
    </select>
  </SettingRow>

  {#if ttsConfig.engine === "device"}
    <SettingRow compact={compact} label="Narration Voice">
      <select class="select select-sm w-52" bind:value={ttsConfig.voice} onchange={save}>
        <option value="">System default</option>
        {#each voices as voice (voice.voiceURI)}
          <option value={voice.voiceURI}>{voice.name} ({voice.lang})</option>
        {/each}
      </select>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice">
      <select class="select select-sm w-52" bind:value={ttsConfig.dialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each voices as voice (voice.voiceURI)}
          <option value={voice.voiceURI}>{voice.name} ({voice.lang})</option>
        {/each}
      </select>
    </SettingRow>
    {#if voices.length === 0}
      <p class="text-xs text-base-content/60">No system voices have been listed yet. Install a voice for your language in the operating system's speech settings.</p>
    {/if}
  {:else if ttsConfig.engine === "edge"}
    <SettingRow compact={compact} label="Narration Voice">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" bind:value={ttsConfig.edgeVoice} onchange={save}>
          {#if edgeVoices.length === 0}
            <option value={ttsConfig.edgeVoice}>{ttsConfig.edgeVoice || "Load voices"}</option>
          {/if}
          {#each edgeVoices as voice (voice)}
            <option value={voice}>{voice}</option>
          {/each}
        </select>
        <button class="btn btn-outline btn-sm" onclick={refreshEdgeVoices} disabled={edgeVoiceLoading}>
          {edgeVoiceLoading ? "Loading…" : "Refresh"}
        </button>
      </div>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice">
      <select class="select select-sm w-52" bind:value={ttsConfig.edgeDialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each edgeVoices as voice (voice)}
          <option value={voice}>{voice}</option>
        {/each}
      </select>
    </SettingRow>
    {#if edgeVoiceError}
      <p class="text-xs text-error">{edgeVoiceError}</p>
    {/if}
    <p class="text-xs text-base-content/60">Edge TTS uses Microsoft's Read Aloud service. It does not require an API key.</p>
  {:else if ttsConfig.engine === "azure"}
    <SettingRow compact={compact} label="Azure Region">
      <input class="input input-sm w-52" bind:value={ttsConfig.azureRegion} onchange={save} placeholder="japaneast" />
    </SettingRow>
    <SettingRow compact={compact} label="Subscription Key">
      <input class="input input-sm w-full max-w-lg" type="password" autocomplete="off" bind:value={ttsRuntime.azureKey} placeholder="Azure Speech key (session only)" />
    </SettingRow>
    <SettingRow compact={compact} label="Narration Voice">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" bind:value={ttsConfig.azureVoice} onchange={save}>
          {#if azureVoices.length === 0}<option value={ttsConfig.azureVoice}>{ttsConfig.azureVoice}</option>{/if}
          {#each azureVoices as voice (voice)}<option value={voice}>{voice}</option>{/each}
        </select>
        <button class="btn btn-outline btn-sm" onclick={refreshAzureVoices} disabled={azureVoiceLoading}>{azureVoiceLoading ? "Loading…" : "Refresh"}</button>
      </div>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice">
      <select class="select select-sm w-52" bind:value={ttsConfig.azureDialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each azureVoices as voice (voice)}<option value={voice}>{voice}</option>{/each}
      </select>
    </SettingRow>
    {#if azureVoiceError}<p class="text-xs text-error">{azureVoiceError}</p>{/if}
  {:else if ttsConfig.engine === "microsoftTranslator"}
    <SettingRow compact={compact} label="Narration Voice">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" bind:value={ttsConfig.microsoftTranslatorVoice} onchange={save}>
          {#if translatorVoices.length === 0}<option value={ttsConfig.microsoftTranslatorVoice}>{ttsConfig.microsoftTranslatorVoice}</option>{/if}
          {#each translatorVoices as voice (voice)}<option value={voice}>{voice}</option>{/each}
        </select>
        <button class="btn btn-outline btn-sm" onclick={refreshTranslatorVoices} disabled={translatorVoiceLoading}>{translatorVoiceLoading ? "Loading…" : "Refresh"}</button>
      </div>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice">
      <select class="select select-sm w-52" bind:value={ttsConfig.microsoftTranslatorDialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each translatorVoices as voice (voice)}<option value={voice}>{voice}</option>{/each}
      </select>
    </SettingRow>
    {#if translatorVoiceError}<p class="text-xs text-error">{translatorVoiceError}</p>{/if}
    <p class="text-xs text-base-content/60">Uses Microsoft's Translator mobile-client service. Availability and access can change.</p>
  {:else if ttsConfig.engine === "googleCloud"}
    <SettingRow compact={compact} label="API Key">
      <input class="input input-sm w-full max-w-lg" type="password" autocomplete="off" bind:value={ttsRuntime.googleCloudKey} placeholder="Google Cloud key (session only)" />
    </SettingRow>
    <SettingRow compact={compact} label="Narration Voice">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" bind:value={ttsConfig.googleCloudVoice} onchange={save}>
          {#if googleCloudVoices.length === 0}<option value={ttsConfig.googleCloudVoice}>{ttsConfig.googleCloudVoice}</option>{/if}
          {#each googleCloudVoices as voice (voice)}<option value={voice}>{voice}</option>{/each}
        </select>
        <button class="btn btn-outline btn-sm" onclick={refreshGoogleCloudVoices} disabled={googleCloudVoiceLoading}>{googleCloudVoiceLoading ? "Loading…" : "Refresh"}</button>
      </div>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice">
      <select class="select select-sm w-52" bind:value={ttsConfig.googleCloudDialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each googleCloudVoices as voice (voice)}<option value={voice}>{voice}</option>{/each}
      </select>
    </SettingRow>
    {#if googleCloudVoiceError}<p class="text-xs text-error">{googleCloudVoiceError}</p>{/if}
  {:else if ttsConfig.engine === "googleTranslate"}
    <p class="text-xs text-base-content/60">Google Translate uses its default Japanese voice. It has no voice selector or API key.</p>
  {:else if ttsConfig.engine === "openAi"}
    <SettingRow compact={compact} label="Base URL">
      <input
        class="input input-sm w-full max-w-lg"
        type="url"
        placeholder={ttsConfig.engine === "openAi" ? "https://api.openai.com/v1" : "http://127.0.0.1:5000"}
        bind:value={ttsConfig.endpoint}
        onchange={save}
      />
    </SettingRow>
    <SettingRow compact={compact} label="API Key">
      <input class="input input-sm w-full max-w-lg" type="password" autocomplete="off" bind:value={ttsRuntime.apiKey} placeholder="Optional; held in memory only" />
    </SettingRow>
    {#if ttsConfig.engine === "openAi"}
      <SettingRow compact={compact} label="Model">
        <input class="input input-sm w-52" bind:value={ttsConfig.model} onchange={save} placeholder="tts-1" />
      </SettingRow>
      <SettingRow compact={compact} label="Narration Voice">
        <input class="input input-sm w-52" bind:value={ttsConfig.remoteVoice} onchange={save} placeholder="alloy" />
      </SettingRow>
      <SettingRow compact={compact} label="Dialogue Voice">
        <input class="input input-sm w-52" bind:value={ttsConfig.remoteDialogueVoice} onchange={save} placeholder="Same as narration" />
      </SettingRow>
    {/if}
    <p class="text-xs text-base-content/60">Generated speech text is sent to the configured service. HTTPS is required for remote hosts; HTTP is allowed only for localhost or private-network hosts. API keys are not saved to disk and are cleared when the app restarts.</p>
  {/if}
</SettingsSection>

<SettingsSection compact={compact} title="Playback">
  <SettingRow compact={compact} label="Speech Rate">
    {#if ttsConfig.engine === "googleTranslate"}
      <select class="select select-sm w-36" value={Number(ttsConfig.rate) < 0.75 ? "slow" : "normal"} onchange={(event) => { ttsConfig.rate = event.currentTarget.value === "slow" ? 0.5 : 1; save(); }}>
        <option value="normal">Normal</option>
        <option value="slow">Slow</option>
      </select>
    {:else}
      <div class="flex w-56 items-center gap-3">
        <input
          class="range range-xs flex-1"
          type="range"
          min={ttsConfig.engine === "googleCloud" ? 0.25 : 0.5}
          max={["azure", "microsoftTranslator", "googleCloud"].includes(ttsConfig.engine) ? 2 : 3}
          step={0.1}
          bind:value={ttsConfig.rate}
          oninput={save}
        />
        <span class="w-10 text-right text-xs tabular-nums">{Number(ttsConfig.rate).toFixed(1)}x</span>
      </div>
    {/if}
  </SettingRow>
  {#if ["device", "edge", "azure", "microsoftTranslator", "googleCloud"].includes(ttsConfig.engine)}
    <SettingRow compact={compact} label={ttsConfig.engine === "edge" ? "Pitch (Hz)" : ttsConfig.engine === "device" ? "Pitch" : "Pitch (semitones)"}>
      <div class="flex w-56 items-center gap-3">
        <input
          class="range range-xs flex-1"
          type="range"
          min={ttsConfig.engine === "edge" ? -50 : ttsConfig.engine === "device" ? 0.5 : ["googleCloud"].includes(ttsConfig.engine) ? -20 : -12}
          max={ttsConfig.engine === "edge" ? 50 : ttsConfig.engine === "device" ? 2 : ttsConfig.engine === "googleCloud" ? 20 : 7}
          step={ttsConfig.engine === "edge" ? 1 : ttsConfig.engine === "device" ? 0.1 : 1}
          bind:value={ttsConfig.pitch}
          oninput={save}
        />
        <span class="w-12 text-right text-xs tabular-nums">
          {ttsConfig.engine === "edge"
            ? `${Number(ttsConfig.pitch) > 0 ? "+" : ""}${Number(ttsConfig.pitch).toFixed(0)} Hz`
            : ttsConfig.engine === "device"
              ? Number(ttsConfig.pitch).toFixed(1)
              : `${Number(ttsConfig.pitch) > 0 ? "+" : ""}${Number(ttsConfig.pitch).toFixed(0)} st`}
        </span>
      </div>
    </SettingRow>
  {/if}
  <SettingRow compact={compact} label="Playback Speed">
    <div class="flex w-56 items-center gap-3">
      <input class="range range-xs flex-1" type="range" min="0.5" max="3" step="0.1" bind:value={ttsConfig.playbackRate} oninput={save} />
      <span class="w-10 text-right text-xs tabular-nums">{Number(ttsConfig.playbackRate).toFixed(1)}x</span>
    </div>
  </SettingRow>
  <SettingRow compact={compact} label="Volume">
    <div class="flex w-56 items-center gap-3">
      <input class="range range-xs flex-1" type="range" min="0" max="1" step="0.05" bind:value={ttsConfig.volume} oninput={save} />
      <span class="w-10 text-right text-xs tabular-nums">{Math.round(Number(ttsConfig.volume) * 100)}%</span>
    </div>
  </SettingRow>
  <SettingRow compact={compact} label="Separate Dialogue Voice">
    <input class="toggle toggle-sm toggle-primary" type="checkbox" bind:checked={ttsConfig.splitDialogue} onchange={save} disabled={ttsConfig.engine === "googleTranslate"} />
  </SettingRow>
  <SettingRow compact={compact} label="Use Ruby Readings">
    <input class="toggle toggle-sm toggle-primary" type="checkbox" bind:checked={ttsConfig.useRubyReadings} onchange={save} />
  </SettingRow>
  <p class="text-xs text-base-content/60">Dialogue splitting uses Japanese quote marks and routes quoted text to the optional dialogue voice. Furigana readings replace ruby text where EPUB markup provides an rt reading.</p>
</SettingsSection>

<SettingsSection compact={compact} title="Reader Playback">
  <SettingToggle
    label="Reader Auto-Scroll"
    {compact}
    bind:checked={sasayakiConfig.sasayakiAutoScroll}
    onchange={saveSasayakiConfig}
  />
  <SettingToggle
    label="Auto-Pause on Lookup"
    {compact}
    bind:checked={sasayakiConfig.sasayakiAutoPause}
    onchange={saveSasayakiConfig}
  />
  <SettingRow compact={compact} label="Image Hold">
    <div class="flex w-56 items-center gap-3">
      <input
        class="range range-xs flex-1"
        type="range"
        min="0"
        max="5"
        step="0.5"
        value={sasayakiConfig.sasayakiImagePause ? Math.min(5, Math.max(0, Number(sasayakiConfig.sasayakiImagePauseDuration) || 0)) : 0}
        oninput={(event) => {
          const seconds = Number(event.currentTarget.value);
          sasayakiConfig.sasayakiImagePauseDuration = seconds;
          sasayakiConfig.sasayakiImagePause = seconds > 0;
          saveSasayakiConfig();
        }}
      />
      <span class="w-12 text-right text-xs tabular-nums">{sasayakiConfig.sasayakiImagePause ? `${Number(sasayakiConfig.sasayakiImagePauseDuration).toFixed(1)}s` : "Off"}</span>
    </div>
  </SettingRow>
  <p class="text-xs text-base-content/60">Auto-scroll follows the sentence being read. Image hold pauses speech on illustrated pages; auto-scroll must be enabled for image holds to run.</p>
</SettingsSection>

<SettingsSection compact={compact} title="Pronunciation Replacements">
  <p class="mb-1 text-xs text-base-content/60">Rules are applied in order to the book text before it is spoken. Patterns are JavaScript regular expressions. Online providers receive the text being synthesized.</p>
  {#each ttsConfig.replacements as rule (rule.id)}
    <div class="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-2">
      <input class="input input-sm min-w-0" bind:value={rule.pattern} onchange={save} placeholder="Pattern (regex)" aria-label="Pronunciation pattern" />
      <input class="input input-sm min-w-0" bind:value={rule.replacement} onchange={save} placeholder="Replacement" aria-label="Pronunciation replacement" />
      <button class="btn btn-ghost btn-sm btn-square text-error" title="Remove rule" onclick={() => removeReplacement(rule.id)}>×</button>
    </div>
  {/each}
  <div>
    <button class="btn btn-outline btn-sm" onclick={addReplacement}>Add replacement</button>
  </div>
</SettingsSection>
