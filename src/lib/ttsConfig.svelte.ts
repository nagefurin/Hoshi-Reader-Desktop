import { persisted } from "./persisted.svelte";

export type TtsEngine = "device" | "edge" | "googleTranslate" | "openAi" | "vits";
export type TtsReplacement = {
  id: string;
  pattern: string;
  replacement: string;
};

export type TtsConfig = {
  engine: TtsEngine;
  voice: string;
  dialogueVoice: string;
  splitDialogue: boolean;
  useRubyReadings: boolean;
  rate: number;
  pitch: number;
  volume: number;
  endpoint: string;
  model: string;
  remoteVoice: string;
  vitsModel: "vits" | "w2v2-vits" | "bert-vits2" | "gpt-sovits";
  speakerId: number;
  replacements: TtsReplacement[];
};

const defaults: TtsConfig = {
  engine: "device",
  voice: "",
  dialogueVoice: "",
  splitDialogue: true,
  useRubyReadings: false,
  rate: 1,
  pitch: 1,
  volume: 1,
  endpoint: "",
  model: "tts-1",
  remoteVoice: "alloy",
  vitsModel: "vits",
  speakerId: 0,
  replacements: [],
};

const store = persisted<TtsConfig>("tts.config", defaults);
export const ttsConfig = store.config;
export const saveTtsConfig = store.save;

// Secrets are deliberately kept out of persisted settings and cleared when the app is restarted.
export const ttsRuntime = $state({ apiKey: "" });
