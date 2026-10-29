<script lang="ts">
  import { ttsConfig, ttsRuntime, saveTtsConfig } from "./ttsConfig.svelte";
  import SettingsSection from "./SettingsSection.svelte";
  import SettingRow from "./SettingRow.svelte";

  let voices = $state<SpeechSynthesisVoice[]>([]);

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

<SettingsSection title="Speech Engine">
  <SettingRow label="Provider">
    <select class="select select-sm w-52" bind:value={ttsConfig.engine} onchange={save}>
      <option value="device">Device / system voice</option>
      <option value="googleTranslate">Google Translate (online)</option>
      <option value="openAi">OpenAI-compatible endpoint</option>
      <option value="vits">VITS (self-hosted)</option>
    </select>
  </SettingRow>

  {#if ttsConfig.engine === "device"}
    <SettingRow label="Narration Voice">
      <select class="select select-sm w-52" bind:value={ttsConfig.voice} onchange={save}>
        <option value="">System default</option>
        {#each voices as voice (voice.voiceURI)}
          <option value={voice.voiceURI}>{voice.name} ({voice.lang})</option>
        {/each}
      </select>
    </SettingRow>
    <SettingRow label="Dialogue Voice">
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
  {:else if ttsConfig.engine === "openAi" || ttsConfig.engine === "vits"}
    <SettingRow label="Base URL">
      <input
        class="input input-sm w-full max-w-lg"
        type="url"
        placeholder={ttsConfig.engine === "openAi" ? "https://api.openai.com/v1" : "http://127.0.0.1:5000"}
        bind:value={ttsConfig.endpoint}
        onchange={save}
      />
    </SettingRow>
    <SettingRow label="API Key">
      <input class="input input-sm w-full max-w-lg" type="password" autocomplete="off" bind:value={ttsRuntime.apiKey} placeholder="Optional; held in memory only" />
    </SettingRow>
    {#if ttsConfig.engine === "openAi"}
      <SettingRow label="Model">
        <input class="input input-sm w-52" bind:value={ttsConfig.model} onchange={save} placeholder="tts-1" />
      </SettingRow>
      <SettingRow label="Voice">
        <input class="input input-sm w-52" bind:value={ttsConfig.remoteVoice} onchange={save} placeholder="alloy" />
      </SettingRow>
    {:else}
      <SettingRow label="VITS Model">
        <select class="select select-sm w-52" bind:value={ttsConfig.vitsModel} onchange={save}>
          <option value="vits">VITS</option>
          <option value="w2v2-vits">W2V2-VITS</option>
          <option value="bert-vits2">BERT-VITS2</option>
          <option value="gpt-sovits">GPT-SoVITS</option>
        </select>
      </SettingRow>
      <SettingRow label="Speaker ID">
        <input class="input input-sm w-28" type="number" min="0" step="1" bind:value={ttsConfig.speakerId} onchange={save} />
      </SettingRow>
    {/if}
    <p class="text-xs text-base-content/60">Generated speech text is sent to the configured service. HTTPS is required for remote hosts; HTTP is allowed only for localhost or private-network hosts. API keys are not saved to disk and are cleared when the app restarts.</p>
  {/if}
</SettingsSection>

<SettingsSection title="Playback">
  <SettingRow label="Speech Rate">
    <div class="flex w-56 items-center gap-3">
      <input class="range range-xs flex-1" type="range" min="0.5" max="3" step="0.1" bind:value={ttsConfig.rate} oninput={save} />
      <span class="w-10 text-right text-xs tabular-nums">{Number(ttsConfig.rate).toFixed(1)}x</span>
    </div>
  </SettingRow>
  {#if ttsConfig.engine === "device"}
    <SettingRow label="Pitch">
      <div class="flex w-56 items-center gap-3">
        <input class="range range-xs flex-1" type="range" min="0.5" max="2" step="0.1" bind:value={ttsConfig.pitch} oninput={save} />
        <span class="w-10 text-right text-xs tabular-nums">{Number(ttsConfig.pitch).toFixed(1)}</span>
      </div>
    </SettingRow>
  {/if}
  <SettingRow label="Volume">
    <div class="flex w-56 items-center gap-3">
      <input class="range range-xs flex-1" type="range" min="0" max="1" step="0.05" bind:value={ttsConfig.volume} oninput={save} />
      <span class="w-10 text-right text-xs tabular-nums">{Math.round(Number(ttsConfig.volume) * 100)}%</span>
    </div>
  </SettingRow>
  <SettingRow label="Dialogue Voice">
    <input class="toggle toggle-sm toggle-primary" type="checkbox" bind:checked={ttsConfig.splitDialogue} onchange={save} disabled={ttsConfig.engine !== "device"} />
  </SettingRow>
  <SettingRow label="Use Furigana Readings">
    <input class="toggle toggle-sm toggle-primary" type="checkbox" bind:checked={ttsConfig.useRubyReadings} onchange={save} />
  </SettingRow>
  <p class="text-xs text-base-content/60">Dialogue splitting uses Japanese quote marks and routes quoted text to the optional dialogue voice. Furigana readings replace ruby text where EPUB markup provides an rt reading.</p>
</SettingsSection>

<SettingsSection title="Pronunciation Replacements">
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
