import { persisted } from "./persisted.svelte";

export type TtsEngine = "device" | "edge" | "googleTranslate" | "azure" | "microsoftTranslator" | "googleCloud" | "openAi";
export type TtsReplacement = {
  id: string;
  pattern: string;
  replacement: string;
  regex?: boolean;
  bookId?: string;
};

export type TtsSynthesisOptions = {
  volume: number;
  style: string;
  styleDegree: number;
  role: string;
  leadingMs: number;
  trailingMs: number;
  commaMs: number;
  semicolonMs: number;
  enumerationCommaMs: number;
  sentenceBoundaryMs: number;
  format: "mp3" | "wav";
  sampleRate: 16000 | 24000 | 48000;
  effectsProfile: string;
};

const defaultSynthesisOptions: TtsSynthesisOptions = {
  volume: 0,
  style: "",
  styleDegree: 1,
  role: "",
  leadingMs: -1,
  trailingMs: -1,
  commaMs: -1,
  semicolonMs: -1,
  enumerationCommaMs: -1,
  sentenceBoundaryMs: -1,
  format: "mp3",
  sampleRate: 24000,
  effectsProfile: "",
};

const providerOptions = () => ({
  edge: { ...defaultSynthesisOptions },
  azure: { ...defaultSynthesisOptions },
  microsoftTranslator: { ...defaultSynthesisOptions },
  googleCloud: { ...defaultSynthesisOptions },
  googleTranslate: { ...defaultSynthesisOptions },
  openAi: { ...defaultSynthesisOptions },
});

export type TtsConfig = {
  engine: TtsEngine;
  voice: string;
  dialogueVoice: string;
  splitDialogue: boolean;
  useRubyReadings: boolean;
  rate: number;
  playbackRate: number;
  sentencePauseMs: number;
  prefetchSentences: number;
  pitch: number;
  volume: number;
  endpoint: string;
  model: string;
  remoteVoice: string;
  edgeVoice: string;
  edgeDialogueVoice: string;
  remoteDialogueVoice: string;
  azureRegion: string;
  azureVoice: string;
  azureDialogueVoice: string;
  microsoftTranslatorVoice: string;
  microsoftTranslatorDialogueVoice: string;
  googleCloudVoice: string;
  googleCloudDialogueVoice: string;
  providerOptions: Record<string, TtsSynthesisOptions>;
  replacements: TtsReplacement[];
};

const defaults: TtsConfig = {
  engine: "device",
  voice: "",
  dialogueVoice: "",
  splitDialogue: true,
  useRubyReadings: true,
  rate: 1,
  playbackRate: 1,
  sentencePauseMs: 0,
  prefetchSentences: 5,
  pitch: 1,
  volume: 1,
  endpoint: "",
  model: "tts-1",
  remoteVoice: "alloy",
  edgeVoice: "ja-JP-NanamiNeural",
  edgeDialogueVoice: "",
  remoteDialogueVoice: "",
  azureRegion: "japaneast",
  azureVoice: "ja-JP-NanamiNeural",
  azureDialogueVoice: "",
  microsoftTranslatorVoice: "ja-JP-NanamiNeural",
  microsoftTranslatorDialogueVoice: "",
  googleCloudVoice: "ja-JP-Neural2-B",
  googleCloudDialogueVoice: "",
  providerOptions: providerOptions(),
  replacements: [],
};

const store = persisted<TtsConfig>("tts.config", defaults);
export const ttsConfig = store.config;
export const saveTtsConfig = store.save;

// Secrets are deliberately kept out of persisted settings and cleared when the app is restarted.
export const ttsRuntime = $state({ apiKey: "", azureKey: "", googleCloudKey: "" });
