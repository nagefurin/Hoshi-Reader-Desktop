import { persisted } from "./persisted.svelte";

export type TtsEngine = "device" | "edge" | "googleTranslate" | "azure" | "microsoftTranslator" | "googleCloud" | "openAi";
export type TtsReplacement = {
  id: string;
  pattern: string;
  replacement: string;
  regex?: boolean;
  bookId?: string;
};

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
  replacements: [],
};

const store = persisted<TtsConfig>("tts.config", defaults);
export const ttsConfig = store.config;
export const saveTtsConfig = store.save;

// Secrets are deliberately kept out of persisted settings and cleared when the app is restarted.
export const ttsRuntime = $state({ apiKey: "", azureKey: "", googleCloudKey: "" });
