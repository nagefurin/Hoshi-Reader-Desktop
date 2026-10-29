<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { AudioLines, Pause, Play, Repeat, SkipBack, SkipForward, Square } from "@lucide/svelte";
  import { ttsConfig, ttsRuntime } from "./ttsConfig.svelte";
  import TtsSettings from "./TtsSettings.svelte";
  import { sasayakiConfig } from "./sasayakiConfig.svelte";

  type TtsChapter = {
    spineIndex: number;
    title: string;
    text: string;
    rubyText: string;
    imageOffsets: number[];
    imageIndexStart: number;
  };

  type SpokenSegment = {
    chapterIndex: number;
    chapterTitle: string;
    text: string;
    sourceText: string;
    imagesBefore: number[];
    dialogue: boolean;
  };

  let {
    id,
    title,
    currentChapterIndex,
    lookupActive = false,
    continuousMode = false,
    onReadSegment,
    onClearHighlight,
  }: {
    id: string;
    title: string;
    currentChapterIndex: number;
    lookupActive?: boolean;
    continuousMode?: boolean;
    onReadSegment: (spineIndex: number, sourceText: string, imagesBefore: number[]) => Promise<void>;
    onClearHighlight: () => void;
  } = $props();

  let chapters = $state<TtsChapter[]>([]);
  let segments = $state<SpokenSegment[]>([]);
  let position = $state(0);
  let active = $state(false);
  let paused = $state(false);
  let loading = $state(false);
  let errorMessage = $state("");
  let runId = 0;
  let currentAudio: HTMLAudioElement | null = null;
  let currentUtterance: SpeechSynthesisUtterance | null = null;
  let autoPausedForLookup = false;
  let audioCache = new Map<number, { fingerprint: string; promise: Promise<string> }>();
  let pendingNextPosition: number | null = null;
  let waitingBetweenSentences = false;

  $effect(() => {
    const shouldPauseForLookup = lookupActive && sasayakiConfig.sasayakiAutoPause;
    if (shouldPauseForLookup && active && !paused && !autoPausedForLookup) {
      autoPausedForLookup = true;
      pausePlayback();
    } else if (!shouldPauseForLookup && autoPausedForLookup) {
      autoPausedForLookup = false;
      if (active && paused) void resumePlayback();
    }
  });

  const currentSegment = $derived(segments[position] ?? null);
  const progressText = $derived(segments.length ? (Math.min(position + 1, segments.length) + " / " + segments.length) : "—");

  function applyReplacements(text: string) {
    let value = text;
    for (const rule of ttsConfig.replacements) {
      if (!rule.pattern || (rule.bookId && rule.bookId !== id)) continue;
      try {
        if (rule.regex ?? true) value = value.replace(new RegExp(rule.pattern, "gu"), rule.replacement);
        else value = value.split(rule.pattern).join(rule.replacement);
      } catch {
        // An invalid regular expression is ignored, leaving the rest of the rules active.
      }
    }
    return value;
  }

  function sentenceParts(text: string): string[] {
    return (text.match(/[^。！？!?…]+[。！？!?…]+[」』”’"']*|[^。！？!?…]+$/gu) ?? [text])
      .map((part) => part.trim())
      .filter(Boolean);
  }

  function splitQuoted(sentence: string): { text: string; dialogue: boolean }[] {
    if (!ttsConfig.splitDialogue) return [{ text: sentence, dialogue: false }];
    const expression = /[「『][^」』]*[」』]|“[^”]*”|"[^"]*"/gu;
    const result: { text: string; dialogue: boolean }[] = [];
    let cursor = 0;
    for (const match of sentence.matchAll(expression)) {
      const start = match.index ?? cursor;
      const before = sentence.slice(cursor, start).trim();
      if (before) result.push({ text: before, dialogue: false });
      const quoted = match[0];
      const inner = quoted.slice(1, -1).trim();
      if (inner) result.push({ text: inner, dialogue: true });
      cursor = start + quoted.length;
    }
    const after = sentence.slice(cursor).trim();
    if (after) result.push({ text: after, dialogue: false });
    return result.length ? result : [{ text: sentence, dialogue: false }];
  }

  function splitLong(text: string): string[] {
    const max = ttsConfig.engine === "googleTranslate" ? 160 : 650;
    const characters = Array.from(text);
    const result: string[] = [];
    while (characters.length > max) {
      let cut = max;
      const preferred = characters.slice(0, max).lastIndexOf("、") + 1;
      if (preferred >= Math.floor(max / 2)) cut = preferred;
      result.push(characters.splice(0, cut).join(""));
    }
    if (characters.length) result.push(characters.join(""));
    return result;
  }

  function buildSegments(values: TtsChapter[], startIndex: number): SpokenSegment[] {
    const result: SpokenSegment[] = [];
    for (const chapter of values) {
      if (chapter.spineIndex < startIndex) continue;
      const sourceText = chapter.text;
      const spokenText = ttsConfig.useRubyReadings ? chapter.rubyText : chapter.text;
      const sourceParagraphs = sourceText.split(/\n+/u);
      const spokenParagraphs = spokenText.split(/\n+/u);
      let sourceSearchFrom = 0;
      let imageCursor = 0;
      const imageOffsets = chapter.imageOffsets ?? [];

      for (let paragraphIndex = 0; paragraphIndex < sourceParagraphs.length; paragraphIndex++) {
        const paragraph = sourceParagraphs[paragraphIndex];
        const spokenParagraph = spokenParagraphs[paragraphIndex] ?? paragraph;
        const sourceParts = splitQuoted(paragraph);
        const spokenParts = splitQuoted(spokenParagraph);

        for (let partIndex = 0; partIndex < sourceParts.length; partIndex++) {
          const part = sourceParts[partIndex];
          const spokenPart = spokenParts[partIndex] ?? part;
          const sourceSentences = sentenceParts(part.text);
          const spokenSentences = sentenceParts(spokenPart.text);

          for (let sentenceIndex = 0; sentenceIndex < sourceSentences.length; sentenceIndex++) {
            const sentence = sourceSentences[sentenceIndex];
            const spokenSentence = spokenSentences[sentenceIndex] ?? sentence;
            const sourceChunks = splitLong(sentence);
            const spokenChunks = splitLong(spokenSentence);
            for (let chunkIndex = 0; chunkIndex < sourceChunks.length; chunkIndex++) {
              const source = sourceChunks[chunkIndex].trim();
              if (!source) continue;
              const found = sourceText.indexOf(source, sourceSearchFrom);
              const sourceStart = found >= 0 ? found : sourceSearchFrom;
              if (found >= 0) sourceSearchFrom = found + source.length;

              // Offsets from Rust count Unicode code points, whereas JS string indices use UTF-16.
              const sourceCodePointStart = Array.from(sourceText.slice(0, sourceStart)).length;
              const imagesBefore: number[] = [];
              while (imageCursor < imageOffsets.length && imageOffsets[imageCursor] <= sourceCodePointStart) {
                imagesBefore.push(continuousMode ? (chapter.imageIndexStart + imageCursor) : imageCursor);
                imageCursor++;
              }

              const speech = sourceChunks.length === 1
                ? (spokenChunks.length === 1 ? spokenChunks[0].trim() : spokenSentence.trim())
                : source;
              result.push({
                chapterIndex: chapter.spineIndex,
                chapterTitle: chapter.title,
                text: applyReplacements(speech),
                sourceText: source,
                imagesBefore,
                dialogue: part.dialogue,
              });
            }
          }
        }
      }
    }
    return result;
  }

  function synthesisVoice(segment: SpokenSegment): string {
    if (ttsConfig.engine === "edge") return segment.dialogue ? (ttsConfig.edgeDialogueVoice || ttsConfig.edgeVoice) : ttsConfig.edgeVoice;
    if (ttsConfig.engine === "azure") return segment.dialogue ? (ttsConfig.azureDialogueVoice || ttsConfig.azureVoice) : ttsConfig.azureVoice;
    if (ttsConfig.engine === "microsoftTranslator") return segment.dialogue
      ? (ttsConfig.microsoftTranslatorDialogueVoice || ttsConfig.microsoftTranslatorVoice)
      : ttsConfig.microsoftTranslatorVoice;
    if (ttsConfig.engine === "googleCloud") return segment.dialogue
      ? (ttsConfig.googleCloudDialogueVoice || ttsConfig.googleCloudVoice)
      : ttsConfig.googleCloudVoice;
    if (ttsConfig.engine === "googleTranslate") return "ja";
    return segment.dialogue ? (ttsConfig.remoteDialogueVoice || ttsConfig.remoteVoice) : ttsConfig.remoteVoice;
  }

  function synthesisFingerprint(): string {
    return JSON.stringify([
      ttsConfig.engine, ttsConfig.endpoint, ttsConfig.model, ttsConfig.rate, ttsConfig.pitch,
      ttsConfig.edgeVoice, ttsConfig.edgeDialogueVoice, ttsConfig.azureRegion,
      ttsConfig.azureVoice, ttsConfig.azureDialogueVoice, ttsConfig.microsoftTranslatorVoice,
      ttsConfig.microsoftTranslatorDialogueVoice, ttsConfig.googleCloudVoice,
      ttsConfig.googleCloudDialogueVoice, ttsConfig.remoteVoice, ttsConfig.remoteDialogueVoice,
      ttsRuntime.apiKey, ttsRuntime.azureKey, ttsRuntime.googleCloudKey,
    ]);
  }

  function synthesizedAudio(segmentIndex: number): Promise<string> {
    const segment = segments[segmentIndex];
    if (!segment) return Promise.reject(new Error("TTS segment is no longer available."));
    const fingerprint = synthesisFingerprint();
    const existing = audioCache.get(segmentIndex);
    if (existing?.fingerprint === fingerprint) return existing.promise;

    let promise: Promise<string>;
    promise = invoke<string>("tts_synthesize", {
      engine: ttsConfig.engine,
      text: segment.text,
      baseUrl: ttsConfig.engine === "azure" ? ttsConfig.azureRegion : ttsConfig.endpoint,
      apiKey: ttsConfig.engine === "azure"
        ? ttsRuntime.azureKey
        : ttsConfig.engine === "googleCloud"
          ? ttsRuntime.googleCloudKey
          : ttsRuntime.apiKey,
      model: ttsConfig.model,
      voice: synthesisVoice(segment),
      rate: Number(ttsConfig.rate) || 1,
      pitch: Number(ttsConfig.pitch) || 0,
    }).catch((error) => {
      if (audioCache.get(segmentIndex)?.promise === promise) audioCache.delete(segmentIndex);
      throw error;
    });
    audioCache.set(segmentIndex, { fingerprint, promise });
    return promise;
  }

  function prefetchAhead(segmentIndex: number) {
    if (ttsConfig.engine === "device") return;
    const count = Math.max(1, Math.min(10, Math.round(Number(ttsConfig.prefetchSentences) || 1)));
    const end = Math.min(segments.length, segmentIndex + 1 + count);
    for (let index = segmentIndex + 1; index < end; index++) {
      void synthesizedAudio(index).catch(() => {});
    }
  }

  async function advanceAfterSegment(segmentIndex: number, token: number) {
    if (token !== runId || !active) return;
    pendingNextPosition = segmentIndex + 1;
    const pauseMs = Math.min(2000, Math.max(0, Math.round(Number(ttsConfig.sentencePauseMs) || 0)));
    waitingBetweenSentences = pauseMs > 0;
    if (pauseMs > 0) await new Promise<void>((resolve) => setTimeout(resolve, pauseMs));
    waitingBetweenSentences = false;
    if (token !== runId || !active || paused || pendingNextPosition !== segmentIndex + 1) return;
    position = pendingNextPosition;
    pendingNextPosition = null;
    void speakAt(position, token);
  }

  function cancelCurrent(): number {
    runId += 1;
    pendingNextPosition = null;
    waitingBetweenSentences = false;
    if (typeof window !== "undefined" && "speechSynthesis" in window) window.speechSynthesis.cancel();
    if (currentAudio) {
      currentAudio.pause();
      currentAudio.onended = null;
      currentAudio.onerror = null;
      currentAudio.removeAttribute("src");
      currentAudio.load();
      currentAudio = null;
    }
    if (currentUtterance) {
      currentUtterance.onend = null;
      currentUtterance.onerror = null;
      currentUtterance = null;
    }
    return runId;
  }

  async function playFrom(startIndex: number) {
    const token = cancelCurrent();
    active = false;
    paused = false;
    loading = true;
    errorMessage = "";
    try {
      const loaded = await invoke<TtsChapter[]>("tts_load_text", {
        id,
        useRubyReadings: ttsConfig.useRubyReadings,
      });
      if (token !== runId) return;
      chapters = loaded;
      segments = buildSegments(loaded, startIndex);
      audioCache.clear();
      if (segments.length === 0) throw new Error("No readable text remains after pronunciation replacements.");
      position = 0;
      active = true;
      await speakAt(position, token);
    } catch (error) {
      if (token === runId) {
        errorMessage = String(error);
        active = false;
        paused = false;
      }
    } finally {
      if (token === runId) loading = false;
    }
  }

  function next() {
    if (!segments.length) return;
    jumpTo(Math.min(position + 1, segments.length - 1));
  }

  function previous() {
    if (!segments.length) return;
    jumpTo(Math.max(position - 1, 0));
  }

  function jumpTo(nextPosition: number) {
    const token = cancelCurrent();
    position = nextPosition;
    active = true;
    paused = false;
    errorMessage = "";
    void speakAt(position, token);
  }

  async function speakAt(segmentIndex: number, token: number) {
    if (token !== runId || !active) return;
    if (segmentIndex >= segments.length) {
      active = false;
      paused = false;
      onClearHighlight();
      return;
    }
    const segment = segments[segmentIndex];
    await onReadSegment(segment.chapterIndex, segment.sourceText, segment.imagesBefore);
    if (token !== runId || !active) return;

    if (ttsConfig.engine === "device") {
      if (typeof window === "undefined" || !("speechSynthesis" in window)) {
        errorMessage = "System speech synthesis is not available in this webview.";
        active = false;
        return;
      }
      const utterance = new SpeechSynthesisUtterance(segment.text);
      const availableVoices = window.speechSynthesis.getVoices();
      const voiceUri = segment.dialogue && ttsConfig.dialogueVoice ? ttsConfig.dialogueVoice : ttsConfig.voice;
      if (voiceUri) utterance.voice = availableVoices.find((voice) => voice.voiceURI === voiceUri) ?? null;
      utterance.rate = Math.min(3, Math.max(0.5, (Number(ttsConfig.rate) || 1) * (Number(ttsConfig.playbackRate) || 1)));
      utterance.pitch = Math.min(2, Math.max(0.5, Number(ttsConfig.pitch) || 1));
      utterance.volume = Math.min(1, Math.max(0, Number(ttsConfig.volume) || 0));
      utterance.onend = () => {
        if (token !== runId || !active) return;
        void advanceAfterSegment(segmentIndex, token);
      };
      utterance.onerror = (event) => {
        if (token !== runId || event.error === "canceled" || event.error === "interrupted") return;
        errorMessage = "System TTS error: " + event.error;
        active = false;
        paused = false;
        onClearHighlight();
      };
      currentUtterance = utterance;
      window.speechSynthesis.speak(utterance);
      if (paused) window.speechSynthesis.pause();
      return;
    }

    try {
      const encoded = await synthesizedAudio(segmentIndex);
      if (token !== runId) return;
      const audio = new Audio("data:audio/mpeg;base64," + encoded);
      audio.volume = Math.min(1, Math.max(0, Number(ttsConfig.volume) || 0));
      audio.playbackRate = Math.min(3, Math.max(0.5, Number(ttsConfig.playbackRate) || 1));
      audio.onended = () => {
        if (token !== runId || !active) return;
        void advanceAfterSegment(segmentIndex, token);
      };
      audio.onerror = () => {
        if (token !== runId) return;
        errorMessage = "The generated audio could not be played.";
        active = false;
        paused = false;
        onClearHighlight();
      };
      currentAudio = audio;
      prefetchAhead(segmentIndex);
      if (!paused) await audio.play();
    } catch (error) {
      if (token === runId) {
        errorMessage = String(error);
        active = false;
        paused = false;
        onClearHighlight();
      }
    }
  }

  function pausePlayback() {
    paused = true;
    if (ttsConfig.engine === "device") {
      if ("speechSynthesis" in window) window.speechSynthesis.pause();
    } else {
      currentAudio?.pause();
    }
  }

  async function resumePlayback() {
    paused = false;
    if (pendingNextPosition !== null && !waitingBetweenSentences) {
      position = pendingNextPosition;
      pendingNextPosition = null;
      void speakAt(position, runId);
      return;
    }
    if (pendingNextPosition !== null && waitingBetweenSentences) return;
    if (ttsConfig.engine === "device") {
      if ("speechSynthesis" in window) window.speechSynthesis.resume();
    } else if (currentAudio) {
      try {
        await currentAudio.play();
      } catch (error) {
        errorMessage = String(error);
        active = false;
        onClearHighlight();
      }
    }
  }

  async function togglePlayback() {
    autoPausedForLookup = false;
    if (!active) {
      await playFrom(currentChapterIndex);
      return;
    }
    if (!paused) pausePlayback();
    else await resumePlayback();
  }

  function stop() {
    cancelCurrent();
    active = false;
    paused = false;
    loading = false;
    onClearHighlight();
  }

  onDestroy(() => {
    stop();
  });
</script>

<div class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
  <div class="flex items-start gap-3">
    <div class="rounded-box bg-primary/10 p-2 text-primary"><AudioLines class="size-5" /></div>
    <div class="min-w-0 flex-1">
      <p class="text-sm font-semibold">Text-to-Speech</p>
      <p class="line-clamp-2 text-xs text-base-content/60">{title}</p>
      <p class="mt-1 text-xs text-base-content/60">
        {ttsConfig.engine === "device" ? "System voice" :
          ttsConfig.engine === "edge" ? "Microsoft Edge TTS" :
          ttsConfig.engine === "azure" ? "Microsoft Azure Speech" :
          ttsConfig.engine === "microsoftTranslator" ? "Microsoft Translator" :
          ttsConfig.engine === "googleCloud" ? "Google Cloud Text-to-Speech" :
          ttsConfig.engine === "googleTranslate" ? "Google Translate" : "OpenAI-compatible / custom"}
      </p>
    </div>
  </div>

  <div class="rounded-box border border-base-300 p-3">
    <p class="text-xs text-base-content/60">Now reading</p>
    <p class="mt-1 text-sm font-medium">{currentSegment?.chapterTitle ?? (chapters.find((chapter) => chapter.spineIndex === currentChapterIndex)?.title ?? "Current chapter")}</p>
    <p class="mt-1 text-xs tabular-nums text-base-content/60">Segment {progressText}</p>
  </div>

  <div class="flex items-center justify-center gap-2">
    <button class="btn btn-ghost btn-square" title="Previous sentence" onclick={previous} disabled={!segments.length || loading}>
      <SkipBack class="size-4" />
    </button>
    <button class="btn btn-ghost btn-square" title="Repeat sentence" onclick={() => jumpTo(position)} disabled={!segments.length || loading}>
      <Repeat class="size-4" />
    </button>
    <button class="btn btn-primary btn-circle" title={paused ? "Resume" : active ? "Pause" : "Start"} onclick={togglePlayback} disabled={loading}>
      {#if active && !paused}
        <Pause class="size-5" />
      {:else}
        <Play class="size-5" />
      {/if}
    </button>
    <button class="btn btn-ghost btn-square" title="Next sentence" onclick={next} disabled={!segments.length || loading}>
      <SkipForward class="size-4" />
    </button>
    <button class="btn btn-ghost btn-square" title="Stop" onclick={stop} disabled={!active && !loading}>
      <Square class="size-4" />
    </button>
  </div>

  <button class="btn btn-outline btn-sm" onclick={() => playFrom(currentChapterIndex)} disabled={loading}>
    Start at current chapter
  </button>
  <button class="btn btn-ghost btn-sm" onclick={() => playFrom(0)} disabled={loading}>
    Start from book beginning
  </button>

  {#if loading}
    <p class="text-xs text-base-content/60">Loading text or generating audio…</p>
  {/if}
  {#if active && !paused}
    <p class="text-xs text-base-content/60">Playback continues when this panel is closed.</p>
  {/if}
  {#if errorMessage}
    <div role="alert" class="alert alert-error">
      <span class="break-words text-xs">{errorMessage}</span>
    </div>
  {/if}
  <details class="collapse collapse-arrow rounded-box border border-base-300 bg-base-100">
    <summary class="collapse-title min-h-0 py-3 text-sm font-medium">Text-to-Speech settings</summary>
    <div class="collapse-content overflow-x-hidden">
      <TtsSettings compact={true} bookId={id} />
    </div>
  </details>
  <p class="text-xs text-base-content/50">The active sentence is highlighted in the book while it is being spoken.</p>
</div>
