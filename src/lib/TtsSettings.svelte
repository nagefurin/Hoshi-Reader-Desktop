<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy } from "svelte";
  import { splitTtsSynthesisText } from "./ttsSegmentation";
  import { ttsConfig, ttsRuntime, saveTtsConfig, type TtsEngine, type TtsSynthesisOptions } from "./ttsConfig.svelte";
  import TtsCustomSources from "./TtsCustomSources.svelte";
  import SettingsSection from "./SettingsSection.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingToggle from "./SettingToggle.svelte";
  import { sasayakiConfig, saveSasayakiConfig } from "./sasayakiConfig.svelte";

  let { compact = false, bookId = "", showEnableToggle = true }: { compact?: boolean; bookId?: string; showEnableToggle?: boolean } = $props();

  type TtsVoiceInfo = { id: string; styles: string[]; roles: string[] };
  type SystemVoiceInfo = { voiceURI: string; name: string; lang: string; default: boolean };
  type NativeSystemVoice = { id: string; name: string; language: string; isDefault: boolean };
  const usesNativeWindowsSpeech = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);
  const audioProfiles = [
    { id: "", label: "None" },
    { id: "wearable-class-device", label: "Wearable" },
    { id: "handset-class-device", label: "Handset" },
    { id: "headphone-class-device", label: "Headphones" },
    { id: "small-bluetooth-speaker-class-device", label: "Small speaker" },
    { id: "medium-bluetooth-speaker-class-device", label: "Medium speaker" },
    { id: "large-home-entertainment-class-device", label: "Home audio" },
    { id: "large-automotive-class-device", label: "Car audio" },
    { id: "telephony-class-application", label: "Telephone" },
  ];
  const providerDefaults: TtsSynthesisOptions = {
    volume: 0, style: "", styleDegree: 1, role: "",
    leadingMs: -1, trailingMs: -1, commaMs: -1, semicolonMs: -1,
    enumerationCommaMs: -1, sentenceBoundaryMs: -1,
    format: "mp3", sampleRate: 24000, effectsProfile: "",
  };

  function currentOptions(): TtsSynthesisOptions {
    const engine = ttsConfig.engine;
    if (!ttsConfig.providerOptions[engine]) {
      ttsConfig.providerOptions[engine] = { ...providerDefaults };
    }
    return ttsConfig.providerOptions[engine];
  }

  function updateOptions(update: Partial<TtsSynthesisOptions>) {
    ttsConfig.providerOptions[ttsConfig.engine] = { ...currentOptions(), ...update };
    saveTtsConfig();
  }

  type PauseKey = "leadingMs" | "trailingMs" | "commaMs" | "semicolonMs" | "enumerationCommaMs" | "sentenceBoundaryMs";
  const pauseControls: { label: string; key: PauseKey }[] = [
    { label: "Leading Pause", key: "leadingMs" },
    { label: "Trailing Pause", key: "trailingMs" },
    { label: "Comma Pause", key: "commaMs" },
    { label: "Semicolon Pause", key: "semicolonMs" },
    { label: "Enumeration Comma Pause", key: "enumerationCommaMs" },
    { label: "Sentence Boundary Pause", key: "sentenceBoundaryMs" },
  ];
  function setPauseValue(key: PauseKey, value: number) {
    updateOptions({ [key]: value });
  }

  const currentVoice = $derived.by(() => {
    const engine = ttsConfig.engine;
    const voiceId = engine === "edge" ? ttsConfig.edgeVoice
      : engine === "azure" ? ttsConfig.azureVoice
      : engine === "microsoftTranslator" ? ttsConfig.microsoftTranslatorVoice
      : engine === "googleCloud" ? ttsConfig.googleCloudVoice : "";
    const list = engine === "edge" ? edgeVoices
      : engine === "azure" ? azureVoices
      : engine === "microsoftTranslator" ? translatorVoices
      : engine === "googleCloud" ? googleCloudVoices : [];
    return list.find((voice) => voice.id === voiceId);
  });

  let voices = $state<SystemVoiceInfo[]>([]);
  let systemVoiceCount = $state(0);
  let systemVoiceSummary = $state("");
  let systemVoiceLoading = $state(false);
  let systemVoiceError = $state("");
  let edgeVoices = $state<TtsVoiceInfo[]>([]);
  let edgeVoiceLoading = $state(false);
  let edgeVoiceError = $state("");
  let azureVoices = $state<TtsVoiceInfo[]>([]);
  let azureVoiceLoading = $state(false);
  let azureVoiceError = $state("");
  let translatorVoices = $state<TtsVoiceInfo[]>([]);
  let translatorVoiceLoading = $state(false);
  let translatorVoiceError = $state("");
  let googleCloudVoices = $state<TtsVoiceInfo[]>([]);
  let googleCloudVoiceLoading = $state(false);
  let googleCloudVoiceError = $state("");

  type VoiceSettingKey =
    | "edgeVoice" | "edgeDialogueVoice" | "azureVoice" | "azureDialogueVoice"
    | "microsoftTranslatorVoice" | "microsoftTranslatorDialogueVoice"
    | "googleCloudVoice" | "googleCloudDialogueVoice";

  function selectVoice(key: VoiceSettingKey, value: string) {
    ttsConfig[key] = value;
    if (key === "azureVoice" || key === "microsoftTranslatorVoice") {
      const list = key === "azureVoice" ? azureVoices : translatorVoices;
      const selected = list.find((voice) => voice.id === value);
      const options = currentOptions();
      const style = selected?.styles.includes(options.style) ? options.style : "";
      const role = style && selected?.roles.includes(options.role) ? options.role : "";
      updateOptions({ style, role });
    }
    saveTtsConfig();
  }

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
      edgeVoices = await invoke<TtsVoiceInfo[]>("tts_edge_voices");
      if (edgeVoices.length && !edgeVoices.some((voice) => voice.id === ttsConfig.edgeVoice)) {
        ttsConfig.edgeVoice = edgeVoices.find((voice) => voice.id === "ja-JP-NanamiNeural")?.id
          ?? edgeVoices[0].id;
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
      azureVoices = await invoke<TtsVoiceInfo[]>("tts_azure_voices", {
        region: ttsConfig.azureRegion,
        apiKey: ttsRuntime.azureKey,
      });
      if (azureVoices.length && !azureVoices.some((voice) => voice.id === ttsConfig.azureVoice)) {
        ttsConfig.azureVoice = azureVoices.find((voice) => voice.id === "ja-JP-NanamiNeural")?.id
          ?? azureVoices[0].id;
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
      translatorVoices = await invoke<TtsVoiceInfo[]>("tts_microsoft_translator_voices");
      if (translatorVoices.length && !translatorVoices.some((voice) => voice.id === ttsConfig.microsoftTranslatorVoice)) {
        ttsConfig.microsoftTranslatorVoice = translatorVoices[0].id;
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
      googleCloudVoices = await invoke<TtsVoiceInfo[]>("tts_google_cloud_voices", {
        apiKey: ttsRuntime.googleCloudKey,
      });
      if (googleCloudVoices.length && !googleCloudVoices.some((voice) => voice.id === ttsConfig.googleCloudVoice)) {
        ttsConfig.googleCloudVoice = googleCloudVoices[0].id;
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

  async function refreshSystemVoices() {
    if (usesNativeWindowsSpeech) {
      systemVoiceLoading = true;
      systemVoiceError = "";
      try {
        const listed = await invoke<NativeSystemVoice[]>("tts_system_voices");
        systemVoiceCount = listed.length;
        systemVoiceSummary = listed.slice(0, 5).map((voice) => `${voice.name} (${voice.language})`).join(", ");
        voices = listed
          .filter((voice) => voice.language.trim().toLowerCase().startsWith("ja"))
          .map((voice) => ({
            voiceURI: voice.id,
            name: voice.name,
            lang: voice.language,
            default: voice.isDefault,
          }))
          .sort((a, b) => a.name.localeCompare(b.name));
        if (listed.length > 0) {
          let changed = false;
          if (ttsConfig.voice && !voices.some((voice) => voice.voiceURI === ttsConfig.voice)) {
            ttsConfig.voice = "";
            changed = true;
          }
          if (ttsConfig.dialogueVoice && !voices.some((voice) => voice.voiceURI === ttsConfig.dialogueVoice)) {
            ttsConfig.dialogueVoice = "";
            changed = true;
          }
          if (changed) saveTtsConfig();
        }
      } catch (error) {
        voices = [];
        systemVoiceCount = 0;
        systemVoiceSummary = "";
        systemVoiceError = String(error);
      } finally {
        systemVoiceLoading = false;
      }
      return;
    }

    if (typeof window === "undefined" || !("speechSynthesis" in window)) return;
    const listed = window.speechSynthesis.getVoices();
    systemVoiceCount = listed.length;
    systemVoiceSummary = listed.slice(0, 5).map((voice) => `${voice.name} (${voice.lang})`).join(", ");
    voices = listed
      .filter((voice) => voice.lang.trim().toLowerCase().startsWith("ja"))
      .map((voice) => ({ voiceURI: voice.voiceURI, name: voice.name, lang: voice.lang, default: voice.default }))
      .sort((a, b) => a.name.localeCompare(b.name));

    if (listed.length > 0) {
      let changed = false;
      if (ttsConfig.voice && !voices.some((voice) => voice.voiceURI === ttsConfig.voice)) {
        ttsConfig.voice = "";
        changed = true;
      }
      if (ttsConfig.dialogueVoice && !voices.some((voice) => voice.voiceURI === ttsConfig.dialogueVoice)) {
        ttsConfig.dialogueVoice = "";
        changed = true;
      }
      if (changed) saveTtsConfig();
    }
  }

  $effect(() => {
    if (usesNativeWindowsSpeech) {
      void refreshSystemVoices();
      return;
    }
    if (typeof window === "undefined" || !("speechSynthesis" in window)) return;
    const speech = window.speechSynthesis;
    void refreshSystemVoices();
    speech.addEventListener("voiceschanged", refreshSystemVoices);
    return () => speech.removeEventListener("voiceschanged", refreshSystemVoices);
  });

  function save() {
    saveTtsConfig();
  }

  function addReplacement() {
    ttsConfig.replacements.push({ id: crypto.randomUUID(), pattern: "", replacement: "", regex: false, bookId });
    save();
  }

  function removeReplacement(id: string) {
    ttsConfig.replacements = ttsConfig.replacements.filter((rule) => rule.id !== id);
    save();
  }

  let previewText = $state("今日はいい天気ですね。");
  let previewResult = $state("");
  let previewError = $state("");
  let previewing = $state(false);
  let previewAudio: HTMLAudioElement | null = null;
  let previewToken = 0;
  let resolvePreviewAudio: (() => void) | null = null;

  function stopPreview() {
    previewToken++;
    if (typeof window !== "undefined" && "speechSynthesis" in window) {
      window.speechSynthesis.cancel();
    }
    if (previewAudio) {
      previewAudio.pause();
      previewAudio.onended = null;
      previewAudio.onerror = null;
      previewAudio.removeAttribute("src");
      previewAudio.load();
      previewAudio = null;
    }
    resolvePreviewAudio?.();
    resolvePreviewAudio = null;
    previewing = false;
  }

  function applyPreviewDictionary(source: string): string {
    const scopedRules = ttsConfig.replacements.filter(
      (rule) => rule.pattern && (!rule.bookId || rule.bookId === bookId),
    );
    const literalRules = scopedRules
      .filter((rule) => !(rule.regex ?? true))
      .map((rule) => ({
        pattern: Array.from(rule.pattern),
        replacement: rule.replacement,
        scoped: Boolean(rule.bookId),
      }))
      .sort((a, b) => b.pattern.length - a.pattern.length || Number(b.scoped) - Number(a.scoped));

    const characters = Array.from(source);
    let result = "";
    let cursor = 0;
    while (cursor < characters.length) {
      const literal = literalRules.find(({ pattern }) =>
        pattern.length > 0 &&
        cursor + pattern.length <= characters.length &&
        characters.slice(cursor, cursor + pattern.length).every((character, index) => character === pattern[index]),
      );
      if (literal) {
        result += literal.replacement;
        cursor += literal.pattern.length;
      } else {
        result += characters[cursor];
        cursor++;
      }
    }

    for (const rule of scopedRules) {
      if (!(rule.regex ?? true)) continue;
      try {
        result = result.replace(new RegExp(rule.pattern, "gu"), rule.replacement);
      } catch {
        // Match TTS playback: invalid regex rules are ignored.
      }
    }
    return result;
  }

  function previewVoiceForCurrentEngine(): string {
    const engine = ttsConfig.engine;
    if (engine.startsWith("custom:")) {
      return ttsConfig.customSources.find((source) => source.id === engine)?.narrationVoice ?? "";
    }
    if (engine === "device") return ttsConfig.voice;
    if (engine === "edge") return ttsConfig.edgeVoice;
    if (engine === "azure") return ttsConfig.azureVoice;
    if (engine === "microsoftTranslator") return ttsConfig.microsoftTranslatorVoice;
    if (engine === "googleCloud") return ttsConfig.googleCloudVoice;
    return "ja";
  }

  async function playPreviewAudio(parts: string[], format: "mp3" | "wav", token: number) {
    const mimeType = format === "wav" ? "audio/wav" : "audio/mpeg";
    for (const part of parts) {
      if (token !== previewToken) return;
      const audio = new Audio("data:" + mimeType + ";base64," + part);
      audio.volume = Math.min(1, Math.max(0, Number(ttsConfig.volume) || 0));
      audio.playbackRate = Math.min(3, Math.max(0.5, Number(ttsConfig.playbackRate) || 1));
      previewAudio = audio;
      await new Promise<void>((resolve, reject) => {
        resolvePreviewAudio = resolve;
        audio.onended = () => {
          if (resolvePreviewAudio === resolve) resolvePreviewAudio = null;
          resolve();
        };
        audio.onerror = () => {
          if (resolvePreviewAudio === resolve) resolvePreviewAudio = null;
          reject(new Error("The preview audio could not be played."));
        };
        void audio.play().catch(reject);
      });
      if (token !== previewToken) return;
      previewAudio = null;
    }
  }

  async function runPreview(testDictionary: boolean) {
    const source = previewText.trim();
    if (!source) return;
    stopPreview();
    const token = previewToken;
    previewError = "";
    previewResult = testDictionary ? applyPreviewDictionary(source) : "";
    const spokenText = testDictionary ? previewResult : source;
    if (!spokenText.trim()) {
      previewError = "The pronunciation rules produced empty speech.";
      return;
    }
    previewing = true;

    try {
      const engine = ttsConfig.engine;
      if (engine === "device" && !usesNativeWindowsSpeech) {
        if (typeof window === "undefined" || !("speechSynthesis" in window)) {
          throw new Error("System speech synthesis is not available in this WebView.");
        }
        const japaneseVoices = window.speechSynthesis.getVoices()
          .filter((voice) => voice.lang.trim().toLowerCase().startsWith("ja"));
        if (!japaneseVoices.length) {
          throw new Error("No Japanese system voice is available. Install a Japanese Windows Speech voice or choose an online provider.");
        }
        const selected = japaneseVoices.find((voice) => voice.voiceURI === ttsConfig.voice)
          ?? japaneseVoices.find((voice) => voice.default)
          ?? japaneseVoices[0];
        await new Promise<void>((resolve, reject) => {
          const utterance = new SpeechSynthesisUtterance(spokenText);
          utterance.lang = "ja-JP";
          utterance.voice = selected;
          utterance.rate = Math.min(3, Math.max(0.5, (Number(ttsConfig.rate) || 1) * (Number(ttsConfig.playbackRate) || 1)));
          utterance.pitch = Math.min(2, Math.max(0.5, Number(ttsConfig.pitch) || 1));
          utterance.volume = Math.min(1, Math.max(0, Number(ttsConfig.volume) || 0));
          utterance.onend = () => resolve();
          utterance.onerror = (event) => reject(new Error("System TTS error: " + event.error));
          window.speechSynthesis.speak(utterance);
        });
        return;
      }

      const customSource = ttsConfig.customSources.find((source) => source.id === engine) ?? null;
      const customKey = ttsRuntime.customKeys[engine] ?? "";
      const apiKey = engine === "azure"
        ? ttsRuntime.azureKey
        : engine === "googleCloud"
          ? ttsRuntime.googleCloudKey
          : "";
      const baseUrl = engine === "azure" ? ttsConfig.azureRegion : "";
      const rate = Number(customSource?.rate ?? ttsConfig.rate) || 1;
      const pitch = Number(ttsConfig.pitch) || 0;
      const voice = previewVoiceForCurrentEngine();
      const options = ttsConfig.providerOptions[engine] ?? providerDefaults;
      const encodedParts: string[] = [];
      for (const part of splitTtsSynthesisText(spokenText, engine)) {
        if (token !== previewToken) return;
        encodedParts.push(await invoke<string>("tts_synthesize", {
          engine: engine === "device" && usesNativeWindowsSpeech ? "system" : engine,
          text: part,
          baseUrl,
          apiKey,
          voice,
          rate,
          pitch,
          options,
          customSource,
          customKey,
        }));
      }

      const format = engine === "device" && usesNativeWindowsSpeech
        ? "wav"
        : engine.startsWith("custom:")
          ? (customSource?.format ?? "mp3")
          : ["azure", "microsoftTranslator", "googleCloud"].includes(engine)
            ? (ttsConfig.providerOptions[engine]?.format ?? "mp3")
            : "mp3";
      await playPreviewAudio(encodedParts, format, token);
    } catch (error) {
      if (token === previewToken) previewError = String(error);
    } finally {
      if (token === previewToken) previewing = false;
    }
  }

  onDestroy(stopPreview);
</script>

{#if showEnableToggle}
  <SettingsSection compact={compact} title="Tts (Text-to-Speech)">
    <SettingToggle
      compact={compact}
      label="Enabled"
      bind:checked={ttsConfig.enableTts}
      onchange={() => {
        if (ttsConfig.enableTts) {
          sasayakiConfig.enableSasayaki = false;
          saveSasayakiConfig();
        }
        saveTtsConfig();
      }}
    />
  </SettingsSection>
{/if}

{#if ttsConfig.enableTts}
<SettingsSection compact={compact} title="Speech Engine">
  <SettingRow compact={compact} label="Provider">
    <select class="select select-sm w-52" value={ttsConfig.engine} onchange={(event) => changeEngine(event.currentTarget.value)}>
      <option value="device">Device / system voice</option>
      <option value="edge">Microsoft Edge (online)</option>
      <option value="azure">Microsoft Azure Speech</option>
      <option value="microsoftTranslator">Microsoft Translator</option>
      <option value="googleCloud">Google Cloud Text-to-Speech</option>
      <option value="googleTranslate">Google Translate (online)</option>
      {#if ttsConfig.customSources.length > 0}
        <optgroup label="Custom online sources">
          {#each ttsConfig.customSources as source (source.id)}
            <option value={source.id}>{source.name || source.baseUrl || source.id}</option>
          {/each}
        </optgroup>
      {/if}
    </select>
  </SettingRow>

  {#if ttsConfig.engine === "device"}
    <SettingRow compact={compact} label="Narration Voice" labelWidth="6.75rem">
      <select class="select select-sm w-52" bind:value={ttsConfig.voice} onchange={save}>
        <option value="">Automatic Japanese voice</option>
        {#each voices as voice (voice.voiceURI)}
          <option value={voice.voiceURI}>{voice.name} ({voice.lang})</option>
        {/each}
      </select>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice" labelWidth="6.75rem">
      <select class="select select-sm w-52" bind:value={ttsConfig.dialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each voices as voice (voice.voiceURI)}
          <option value={voice.voiceURI}>{voice.name} ({voice.lang})</option>
        {/each}
      </select>
    </SettingRow>
    {#if voices.length === 0}
      <div class="flex flex-col items-start gap-2">
        {#if systemVoiceError}
          <p class="text-xs text-error">Could not query Windows native speech voices: {systemVoiceError}</p>
        {:else}
          <p class="text-xs text-warning">
            {systemVoiceLoading
              ? "Loading Windows native speech voices…"
              : systemVoiceCount === 0
                ? "No system voices are listed yet. Refresh after Windows finishes loading speech services."
                : `Windows exposes ${systemVoiceCount} system voice(s), but none is Japanese. Device TTS cannot pronounce Japanese correctly without an installed Japanese speech voice.`}
          </p>
          {#if systemVoiceCount > 0}
            <p class="text-xs text-base-content/60">Voices exposed to Hoshi: {systemVoiceSummary || "Names unavailable"}.</p>
            <p class="text-xs text-base-content/60">
              On Windows, open Settings → Time & language → Language & region → Japanese → Language options, then install the Speech feature if offered. Restart Hoshi and refresh afterward. The Windows-native voice list is queried separately from WebView speech synthesis. Microsoft Edge (online) works without a local voice.
            </p>
          {/if}
        {/if}
        <button class="btn btn-outline btn-xs" onclick={refreshSystemVoices} disabled={systemVoiceLoading}>
          {systemVoiceLoading ? "Refreshing…" : "Refresh system voices"}
        </button>
      </div>
    {/if}
  {:else if ttsConfig.engine === "edge"}
    <SettingRow compact={compact} label="Narration Voice" labelWidth="6.75rem">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" value={ttsConfig.edgeVoice} onchange={(event) => selectVoice("edgeVoice", event.currentTarget.value)}>
          {#if edgeVoices.length === 0}
            <option value={ttsConfig.edgeVoice}>{ttsConfig.edgeVoice || "Load voices"}</option>
          {/if}
          {#each edgeVoices as voice (voice.id)}
            <option value={voice.id}>{voice.id}</option>
          {/each}
        </select>
        <button class="btn btn-outline btn-sm w-[4.5rem] shrink-0" onclick={refreshEdgeVoices} disabled={edgeVoiceLoading}>
          {edgeVoiceLoading ? "Loading…" : "Refresh"}
        </button>
      </div>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice" labelWidth="6.75rem">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" bind:value={ttsConfig.edgeDialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each edgeVoices as voice (voice.id)}
          <option value={voice.id}>{voice.id}</option>
        {/each}
        </select>
        <span aria-hidden="true" class="w-[4.5rem] shrink-0"></span>
      </div>
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
    <SettingRow compact={compact} label="Narration Voice" labelWidth="6.75rem">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" value={ttsConfig.azureVoice} onchange={(event) => selectVoice("azureVoice", event.currentTarget.value)}>
          {#if azureVoices.length === 0}<option value={ttsConfig.azureVoice}>{ttsConfig.azureVoice}</option>{/if}
          {#each azureVoices as voice (voice.id)}<option value={voice.id}>{voice.id}</option>{/each}
        </select>
        <button class="btn btn-outline btn-sm w-[4.5rem] shrink-0" onclick={refreshAzureVoices} disabled={azureVoiceLoading}>{azureVoiceLoading ? "Loading…" : "Refresh"}</button>
      </div>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice" labelWidth="6.75rem">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" bind:value={ttsConfig.azureDialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each azureVoices as voice (voice.id)}<option value={voice.id}>{voice.id}</option>{/each}
        </select>
        <span aria-hidden="true" class="w-[4.5rem] shrink-0"></span>
      </div>
    </SettingRow>
    {#if azureVoiceError}<p class="text-xs text-error">{azureVoiceError}</p>{/if}
  {:else if ttsConfig.engine === "microsoftTranslator"}
    <SettingRow compact={compact} label="Narration Voice" labelWidth="6.75rem">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" value={ttsConfig.microsoftTranslatorVoice} onchange={(event) => selectVoice("microsoftTranslatorVoice", event.currentTarget.value)}>
          {#if translatorVoices.length === 0}<option value={ttsConfig.microsoftTranslatorVoice}>{ttsConfig.microsoftTranslatorVoice}</option>{/if}
          {#each translatorVoices as voice (voice.id)}<option value={voice.id}>{voice.id}</option>{/each}
        </select>
        <button class="btn btn-outline btn-sm w-[4.5rem] shrink-0" onclick={refreshTranslatorVoices} disabled={translatorVoiceLoading}>{translatorVoiceLoading ? "Loading…" : "Refresh"}</button>
      </div>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice" labelWidth="6.75rem">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" bind:value={ttsConfig.microsoftTranslatorDialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each translatorVoices as voice (voice.id)}<option value={voice.id}>{voice.id}</option>{/each}
        </select>
        <span aria-hidden="true" class="w-[4.5rem] shrink-0"></span>
      </div>
    </SettingRow>
    {#if translatorVoiceError}<p class="text-xs text-error">{translatorVoiceError}</p>{/if}
    <p class="text-xs text-base-content/60">Uses Microsoft's Translator mobile-client service. Availability and access can change.</p>
  {:else if ttsConfig.engine === "googleCloud"}
    <SettingRow compact={compact} label="API Key">
      <input class="input input-sm w-full max-w-lg" type="password" autocomplete="off" bind:value={ttsRuntime.googleCloudKey} placeholder="Google Cloud key (session only)" />
    </SettingRow>
    <SettingRow compact={compact} label="Narration Voice" labelWidth="6.75rem">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" value={ttsConfig.googleCloudVoice} onchange={(event) => selectVoice("googleCloudVoice", event.currentTarget.value)}>
          {#if googleCloudVoices.length === 0}<option value={ttsConfig.googleCloudVoice}>{ttsConfig.googleCloudVoice}</option>{/if}
          {#each googleCloudVoices as voice (voice.id)}<option value={voice.id}>{voice.id}</option>{/each}
        </select>
        <button class="btn btn-outline btn-sm w-[4.5rem] shrink-0" onclick={refreshGoogleCloudVoices} disabled={googleCloudVoiceLoading}>{googleCloudVoiceLoading ? "Loading…" : "Refresh"}</button>
      </div>
    </SettingRow>
    <SettingRow compact={compact} label="Dialogue Voice" labelWidth="6.75rem">
      <div class="flex max-w-lg flex-1 items-center gap-2">
        <select class="select select-sm min-w-0 flex-1" bind:value={ttsConfig.googleCloudDialogueVoice} onchange={save}>
        <option value="">Same as narration</option>
        {#each googleCloudVoices as voice (voice.id)}<option value={voice.id}>{voice.id}</option>{/each}
        </select>
        <span aria-hidden="true" class="w-[4.5rem] shrink-0"></span>
      </div>
    </SettingRow>
    {#if googleCloudVoiceError}<p class="text-xs text-error">{googleCloudVoiceError}</p>{/if}
  {:else if ttsConfig.engine === "googleTranslate"}
    <p class="text-xs text-base-content/60">Google Translate uses its default Japanese voice. It has no voice selector or API key.</p>
  {/if}
</SettingsSection>

<TtsCustomSources compact={compact} />

{#if ["azure", "microsoftTranslator", "googleCloud"].includes(ttsConfig.engine)}
  <SettingsSection compact={compact} title="Synthesis">
    {#if ["azure", "microsoftTranslator"].includes(ttsConfig.engine) && (currentVoice?.styles.length ?? 0) > 0}
      <SettingRow compact={compact} label="Speaking Style">
        <select
          class="select select-sm w-52"
          value={currentOptions().style}
          onchange={(event) => updateOptions({ style: event.currentTarget.value, role: event.currentTarget.value ? currentOptions().role : "" })}
        >
          <option value="">Provider default</option>
          {#each currentVoice?.styles ?? [] as style (style)}
            <option value={style}>{style}</option>
          {/each}
        </select>
      </SettingRow>
      {#if currentOptions().style}
        <SettingRow compact={compact} label="Style Degree">
          <div class="flex w-56 items-center gap-3">
            <input class="range range-xs flex-1" type="range" min="0.01" max="2" step="0.01" value={currentOptions().styleDegree} oninput={(event) => updateOptions({ styleDegree: Number(event.currentTarget.value) })} />
            <span class="w-10 text-right text-xs tabular-nums">{Number(currentOptions().styleDegree).toFixed(2)}</span>
          </div>
        </SettingRow>
        {#if (currentVoice?.roles.length ?? 0) > 0}
          <SettingRow compact={compact} label="Speaking Role">
            <select class="select select-sm w-52" value={currentOptions().role} onchange={(event) => updateOptions({ role: event.currentTarget.value })}>
              <option value="">Provider default</option>
              {#each currentVoice?.roles ?? [] as role (role)}
                <option value={role}>{role}</option>
              {/each}
            </select>
          </SettingRow>
        {/if}
      {/if}
    {/if}

    {#if ["azure", "microsoftTranslator"].includes(ttsConfig.engine)}
      {#each pauseControls as pause (pause.key)}
        <SettingRow compact={compact} label={pause.label}>
          <label class="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              class="toggle toggle-sm toggle-primary"
              checked={currentOptions()[pause.key] >= 0}
              onchange={(event) => setPauseValue(pause.key, event.currentTarget.checked ? 300 : -1)}
            />
            {currentOptions()[pause.key] >= 0 ? String(currentOptions()[pause.key]) + " ms" : "Provider default"}
          </label>
        </SettingRow>
        {#if currentOptions()[pause.key] >= 0}
          <div class="flex items-center gap-3">
            <input
              class="range range-xs flex-1"
              type="range"
              min="0"
              max="1000"
              step="10"
              value={currentOptions()[pause.key]}
              oninput={(event) => setPauseValue(pause.key, Number(event.currentTarget.value))}
            />
            <span class="w-12 text-right text-xs tabular-nums">{currentOptions()[pause.key]} ms</span>
          </div>
        {/if}
      {/each}
    {/if}

    {#if ["azure", "microsoftTranslator", "googleCloud"].includes(ttsConfig.engine)}
      <SettingRow compact={compact} label="Output Format">
        <select class="select select-sm w-36" value={currentOptions().format} onchange={(event) => updateOptions({ format: event.currentTarget.value as "mp3" | "wav" })}>
          <option value="mp3">MP3</option>
          <option value="wav">WAV</option>
        </select>
      </SettingRow>
    {/if}

    {#if ["azure", "microsoftTranslator", "googleCloud"].includes(ttsConfig.engine)}
      <SettingRow compact={compact} label="Sample Rate">
        <select class="select select-sm w-36" value={String(currentOptions().sampleRate)} onchange={(event) => updateOptions({ sampleRate: Number(event.currentTarget.value) as 16000 | 24000 | 48000 })}>
          <option value="16000">16 kHz</option>
          <option value="24000">24 kHz</option>
          <option value="48000">48 kHz</option>
        </select>
      </SettingRow>
    {/if}

    {#if ttsConfig.engine === "googleCloud"}
      <SettingRow compact={compact} label="Audio Profile">
        <select class="select select-sm w-52" value={currentOptions().effectsProfile} onchange={(event) => updateOptions({ effectsProfile: event.currentTarget.value })}>
          {#each audioProfiles as profile (profile.id)}
            <option value={profile.id}>{profile.label}</option>
          {/each}
        </select>
      </SettingRow>
    {/if}
  </SettingsSection>
{/if}

<SettingsSection compact={compact} title="Playback">
{#if !["device", "googleTranslate"].includes(ttsConfig.engine) && !ttsConfig.engine.startsWith("custom:")}
  <SettingRow compact={compact} label="Synthesis Volume">
    <div class="flex w-56 items-center gap-3">
      <input
        class="range range-xs flex-1"
        type="range"
        min={ttsConfig.engine === "googleCloud" ? -96 : -100}
        max={ttsConfig.engine === "googleCloud" ? 16 : 100}
        step="1"
        value={currentOptions().volume}
        oninput={(event) => updateOptions({ volume: Number(event.currentTarget.value) })}
      />
      <span class="w-14 text-right text-xs tabular-nums">
        {ttsConfig.engine === "googleCloud" ? Math.round(currentOptions().volume) + " dB" : Math.round(currentOptions().volume) + "%"}
      </span>
    </div>
  </SettingRow>
{/if}
{#if ttsConfig.engine.startsWith("custom:")}
  <SettingRow compact={compact} label="Speech Rate">
    <p class="text-xs text-base-content/60">Configured per source in Custom online TTS sources below.</p>
  </SettingRow>
{:else}
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
          step={0.05}
          bind:value={ttsConfig.rate}
          oninput={save}
        />
        <span class="w-10 text-right text-xs tabular-nums">{Number(ttsConfig.rate).toFixed(2)}x</span>
      </div>
    {/if}
  </SettingRow>
{/if}
  {#if ["device", "edge", "azure", "microsoftTranslator", "googleCloud"].includes(ttsConfig.engine) &&
      !(ttsConfig.engine === "googleCloud" && /Chirp|Journey/i.test(ttsConfig.googleCloudVoice))}
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
      <input class="range range-xs flex-1" type="range" min="0.5" max="3" step="0.05" bind:value={ttsConfig.playbackRate} oninput={save} />
      <span class="w-10 text-right text-xs tabular-nums">{Number(ttsConfig.playbackRate).toFixed(2)}x</span>
    </div>
  </SettingRow>
  <SettingRow compact={compact} label="Preload Sentences">
    <div class="flex w-56 items-center gap-3">
      <input class="range range-xs flex-1" type="range" min="1" max="10" step="1" bind:value={ttsConfig.prefetchSentences} oninput={save} />
      <span class="w-10 text-right text-xs tabular-nums">{Math.round(Number(ttsConfig.prefetchSentences))}</span>
    </div>
  </SettingRow>
  <SettingRow compact={compact} label="Extra Sentence Pause">
    <div class="flex w-56 items-center gap-3">
      <input class="range range-xs flex-1" type="range" min="0" max="2000" step="10" bind:value={ttsConfig.sentencePauseMs} oninput={save} />
      <span class="w-12 text-right text-xs tabular-nums">{Math.round(Number(ttsConfig.sentencePauseMs))} ms</span>
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
  <p class="text-xs text-base-content/60">Japanese paired quote marks define dialogue playback units. This option routes quoted text to the optional dialogue voice. Furigana readings replace ruby text where EPUB markup provides an rt reading.</p>
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

<SettingsSection compact={compact} title="Voice Preview & Pronunciation Test">
  <SettingRow compact={compact} label="Test Text">
    <textarea
      class="textarea textarea-sm min-h-20 w-full max-w-lg"
      rows="2"
      bind:value={previewText}
      placeholder="Enter Japanese text to preview or test dictionary rules"
    ></textarea>
  </SettingRow>
  <div class="flex flex-wrap items-center gap-2">
    <button class="btn btn-outline btn-sm" onclick={() => runPreview(false)} disabled={previewing || !previewText.trim()}>
      Preview Voice
    </button>
    <button class="btn btn-primary btn-sm" onclick={() => runPreview(true)} disabled={previewing || !previewText.trim()}>
      Test Pronunciation Dictionary
    </button>
    {#if previewing}
      <button class="btn btn-ghost btn-sm" onclick={stopPreview}>Stop</button>
    {/if}
  </div>
  {#if previewResult}
    <p class="text-xs text-base-content/60" aria-live="polite">Dictionary output: {previewResult}</p>
  {/if}
  {#if previewError}
    <p class="text-xs text-error" role="alert">{previewError}</p>
  {/if}
  <p class="text-xs text-base-content/60">Preview Voice speaks the text as entered. Test Pronunciation Dictionary applies the active global and current-book rules before speaking it with the selected voice and playback settings.</p>
</SettingsSection>

<SettingsSection compact={compact} title="Pronunciation Dictionary">
  <p class="mb-1 text-xs text-base-content/60">Rules run in order before synthesis. Each rule can replace literal text or use a JavaScript regular expression. Rules can apply globally or only to the current book.</p>
  {#each ttsConfig.replacements as rule (rule.id)}
    <div class="flex flex-col gap-2 rounded-box border border-base-300 p-2">
      <div class="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-2">
        <input class="input input-sm min-w-0" bind:value={rule.pattern} onchange={save} placeholder="Text / regex pattern" aria-label="Pronunciation pattern" />
        <input class="input input-sm min-w-0" bind:value={rule.replacement} onchange={save} placeholder="Reading / replacement" aria-label="Pronunciation replacement" />
        <button class="btn btn-ghost btn-sm btn-square text-error" title="Remove rule" onclick={() => removeReplacement(rule.id)}>×</button>
      </div>
      <div class="flex flex-wrap items-center justify-between gap-3">
        <label class="flex items-center gap-2 text-xs">
          <input
            type="checkbox"
            class="checkbox checkbox-xs"
            checked={rule.regex ?? true}
            onchange={(event) => { rule.regex = event.currentTarget.checked; save(); }}
          />
          Regular expression
        </label>
        <select
          class="select select-xs max-w-full"
          value={rule.bookId ? (bookId && rule.bookId === bookId ? "book" : "other") : "global"}
          onchange={(event) => {
            if (event.currentTarget.value === "book" && bookId) rule.bookId = bookId;
            else if (event.currentTarget.value === "global") rule.bookId = "";
            save();
          }}
        >
          <option value="global">All books</option>
          {#if rule.bookId && (!bookId || rule.bookId !== bookId)}
            <option value="other">Book-specific: {rule.bookId}</option>
          {/if}
          <option value="book" disabled={!bookId}>This book only</option>
        </select>
      </div>
    </div>
  {/each}
  <div>
    <button class="btn btn-outline btn-sm" onclick={addReplacement}>Add dictionary rule</button>
  </div>
</SettingsSection>
{/if}
