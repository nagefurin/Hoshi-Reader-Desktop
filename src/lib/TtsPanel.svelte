<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { AudioLines, Pause, Play, Repeat, SkipBack, SkipForward, Square } from "@lucide/svelte";
  import { ttsConfig, ttsRuntime } from "./ttsConfig.svelte";
  import TtsSettings from "./TtsSettings.svelte";
  import { sasayakiConfig } from "./sasayakiConfig.svelte";
  import { isTtsReaderMatchableCharacter, segmentTtsChapter, splitTtsSynthesisText } from "./ttsSegmentation";

  const usesNativeWindowsSpeech = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

  type TtsChapter = {
    spineIndex: number;
    title: string;
    text: string;
    rubyText: string;
    rubyRanges: TtsRubyRange[];
    imageOffsets: number[];
    imageIndexStart: number;
  };

  type TtsRubyRange = { start: number; end: number; reading: string };

  function countReaderCharacters(value: string): number {
    let count = 0;
    for (const character of Array.from(value)) {
      if (isTtsReaderMatchableCharacter(character)) count++;
    }
    return count;
  }

  function readerCharacterOffset(): number {
    return Math.max(0, Math.round(currentCharacterCount - currentChapterCharacterStart));
  }

  type SpokenSegment = {
    chapterIndex: number;
    chapterTitle: string;
    text: string;
    sourceText: string;
    imagesBefore: number[];
    imagesAfter: number[];
    dialogue: boolean;
    readerCharacterStart: number;
  };

  type TtsControlApi = {
    previous: () => void;
    next: () => void;
    repeat: () => void;
    togglePlayback: () => void;
  };

  let {
    id,
    title,
    currentChapterIndex,
    currentCharacterCount,
    currentChapterCharacterStart,
    lookupActive = false,
    continuousMode = false,
    sessionActive = $bindable(false),
    playing = $bindable(false),
    onReadSegment,
    onHoldImages,
    onControlApi,
    onClearHighlight,
  }: {
    id: string;
    title: string;
    currentChapterIndex: number;
    currentCharacterCount: number;
    currentChapterCharacterStart: number;
    lookupActive?: boolean;
    continuousMode?: boolean;
    sessionActive?: boolean;
    playing?: boolean;
    onReadSegment: (spineIndex: number, sourceText: string, imagesBefore: number[]) => Promise<void>;
    onHoldImages: (spineIndex: number, sourceText: string, images: number[]) => Promise<void>;
    onControlApi?: (api: TtsControlApi | null) => void;
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
  type PendingAudioPlayback = {
    segmentIndex: number;
    token: number;
    encodedParts: string[];
    partIndex: number;
    mimeType: string;
  };
  type PendingSystemPlayback = {
    segmentIndex: number;
    token: number;
    parts: string[];
    partIndex: number;
    voice: SpeechSynthesisVoice;
    rate: number;
    pitch: number;
    volume: number;
  };
  let pendingAudioPlayback: PendingAudioPlayback | null = null;
  let pendingSystemPlayback: PendingSystemPlayback | null = null;
  let autoPausedForLookup = false;
  let audioCache = new Map<number, { fingerprint: string; promise: Promise<string[]> }>();
  let pendingNextPosition: number | null = null;
  let waitingBetweenSentences = false;
  let sessionStartIndex = 0;
  let lastRubySetting = ttsConfig.useRubyReadings;

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

  $effect(() => {
    playing = active && !paused;
  });

  $effect(() => {
    onControlApi?.({ previous, next, repeat: repeatCurrent, togglePlayback });
    return () => onControlApi?.(null);
  });

  const currentSegment = $derived(segments[position] ?? null);
  const progressText = $derived(segments.length ? (Math.min(position + 1, segments.length) + " / " + segments.length) : "—");

  $effect(() => {
    const useRubyReadings = ttsConfig.useRubyReadings;
    untrack(() => {
      if (useRubyReadings === lastRubySetting) return;
      lastRubySetting = useRubyReadings;
      if (!chapters.length) return;

      const previousPosition = position;
      const previousSegment = segments[previousPosition];
      const wasActive = active;
      const wasPaused = paused;
      segments = buildSegments(chapters, sessionStartIndex);
      audioCache.clear();

      const matches = segments
        .map((segment, index) => ({ segment, index }))
        .filter(({ segment }) =>
          previousSegment &&
          segment.chapterIndex === previousSegment.chapterIndex &&
          segment.sourceText === previousSegment.sourceText,
        );
      position = matches.length
        ? matches.reduce((best, current) =>
            Math.abs(current.index - previousPosition) < Math.abs(best.index - previousPosition) ? current : best,
          ).index
        : Math.min(previousPosition, Math.max(segments.length - 1, 0));

      if (wasActive && segments.length) {
        jumpTo(position);
        if (wasPaused) pausePlayback();
      }
    });
  });

  function applyRegexReplacements(text: string) {
    let value = text;
    for (const rule of ttsConfig.replacements) {
      if (!rule.pattern || (rule.bookId && rule.bookId !== id) || !(rule.regex ?? true)) continue;
      try {
        value = value.replace(new RegExp(rule.pattern, "gu"), rule.replacement);
      } catch {
        // An invalid regular expression is ignored, leaving the rest of the rules active.
      }
    }
    return value;
  }

  function speakableSegment(source: string, localRanges: TtsRubyRange[]): string {
    const characters = Array.from(source);
    const literalRules = ttsConfig.replacements
      .filter((rule) => rule.pattern && !(rule.regex ?? true) && (!rule.bookId || rule.bookId === id))
      .map((rule) => ({ pattern: Array.from(rule.pattern), replacement: rule.replacement, scoped: Boolean(rule.bookId) }))
      .sort((a, b) => b.pattern.length - a.pattern.length || Number(b.scoped) - Number(a.scoped));

    const ranges = ttsConfig.useRubyReadings
      ? localRanges
          .filter((range) => Number.isInteger(range.start) && Number.isInteger(range.end) &&
            range.start >= 0 && range.end >= range.start && range.end <= characters.length && range.reading.trim())
          .slice()
          .sort((a, b) => a.start - b.start || a.end - b.end)
      : [];

    // Match Android's precedence: a literal dictionary entry wins over a ruby span
    // if it matches anywhere inside that span; otherwise speak the span's rt reading.
    const blockedRuby = new Set<TtsRubyRange>();
    for (const range of ranges) {
      if (range.end <= range.start) continue;
      const literalInside = literalRules.some(({ pattern }) => {
        if (!pattern.length) return false;
        for (let at = range.start; at < range.end; at++) {
          if (at + pattern.length <= characters.length &&
              characters.slice(at, at + pattern.length).every((character, index) => character === pattern[index])) {
            return true;
          }
        }
        return false;
      });
      if (literalInside) blockedRuby.add(range);
    }

    let value = "";
    let cursor = 0;
    let rubyIndex = 0;
    while (cursor <= characters.length) {
      while (rubyIndex < ranges.length && ranges[rubyIndex].start === cursor && ranges[rubyIndex].end === cursor) {
        value += ranges[rubyIndex].reading;
        rubyIndex++;
      }
      if (cursor >= characters.length) break;
      while (rubyIndex < ranges.length &&
          (ranges[rubyIndex].end < cursor ||
           (ranges[rubyIndex].start < cursor && ranges[rubyIndex].end > cursor))) {
        rubyIndex++;
      }

      const literal = literalRules.find(({ pattern }) =>
        pattern.length > 0 &&
        cursor + pattern.length <= characters.length &&
        characters.slice(cursor, cursor + pattern.length).every((character, index) => character === pattern[index]),
      );
      if (literal) {
        value += literal.replacement;
        cursor += literal.pattern.length;
        continue;
      }

      const ruby = ranges[rubyIndex];
      if (ruby && ruby.start === cursor && ruby.end > cursor && !blockedRuby.has(ruby)) {
        value += ruby.reading;
        cursor = ruby.end;
        rubyIndex++;
        continue;
      }

      value += characters[cursor];
      cursor++;
    }
    return applyRegexReplacements(value);
  }

  function buildSegments(values: TtsChapter[], startIndex: number): SpokenSegment[] {
    const result: SpokenSegment[] = [];
    for (const chapter of values) {
      if (chapter.spineIndex < startIndex) continue;
      const sourceText = chapter.text;
      let imageCursor = 0;
      const imageOffsets = chapter.imageOffsets ?? [];
      const rubyRanges = chapter.rubyRanges ?? [];
      const sourceCodePointCount = Array.from(sourceText).length;
      const firstChapterSegment = result.length;
      // Image-backed ruby glyphs are zero-width source insertions rather than
      // standalone image holds in Android TtsText.
      const rubyImageOffsets = new Set(
        rubyRanges.filter((range) => range.start === range.end).map((range) => range.start),
      );
      const sourceSegments = segmentTtsChapter(sourceText, rubyRanges, imageOffsets);

      for (const span of sourceSegments) {
        const source = sourceText.slice(span.start, span.end);
        if (!source) continue;

        // Backend ruby/image offsets use code points; the segment spans use UTF-16.
        const sourceCodePointStart = Array.from(sourceText.slice(0, span.start)).length;
        const sourceCodePointEnd = sourceCodePointStart + Array.from(source).length;
        const localRubyRanges = rubyRanges
          .filter((range) => {
            if (range.start === range.end) {
              return range.start >= sourceCodePointStart &&
                (range.start < sourceCodePointEnd || sourceCodePointEnd === sourceCodePointCount);
            }
            return range.start >= sourceCodePointStart && range.end <= sourceCodePointEnd;
          })
          .map((range) => ({
            start: range.start - sourceCodePointStart,
            end: range.end - sourceCodePointStart,
            reading: range.reading,
          }));

        const imagesBefore: number[] = [];
        while (imageCursor < imageOffsets.length && imageOffsets[imageCursor] <= sourceCodePointStart) {
          if (!rubyImageOffsets.has(imageOffsets[imageCursor])) {
            imagesBefore.push(continuousMode ? (chapter.imageIndexStart + imageCursor) : imageCursor);
          }
          imageCursor++;
        }

        result.push({
          chapterIndex: chapter.spineIndex,
          chapterTitle: chapter.title,
          text: speakableSegment(source, localRubyRanges),
          sourceText: source,
          imagesBefore,
          imagesAfter: [],
          dialogue: ttsConfig.splitDialogue && span.dialogue,
          readerCharacterStart: countReaderCharacters(sourceText.slice(0, span.start)),
        });
      }

      const trailingImages: number[] = [];
      while (imageCursor < imageOffsets.length) {
        if (!rubyImageOffsets.has(imageOffsets[imageCursor])) {
          trailingImages.push(continuousMode ? (chapter.imageIndexStart + imageCursor) : imageCursor);
        }
        imageCursor++;
      }
      if (trailingImages.length && result.length > firstChapterSegment) {
        result[result.length - 1].imagesAfter.push(...trailingImages);
      }
    }
    return result;
  }

  function synthesisVoice(segment: SpokenSegment): string {
    if (ttsConfig.engine.startsWith("custom:")) {
      const source = ttsConfig.customSources.find((item) => item.id === ttsConfig.engine);
      return segment.dialogue && source?.dialogueVoice ? source.dialogueVoice : (source?.narrationVoice ?? "");
    }
    if (ttsConfig.engine === "device") return segment.dialogue && ttsConfig.dialogueVoice ? ttsConfig.dialogueVoice : ttsConfig.voice;
    if (ttsConfig.engine === "edge") return segment.dialogue ? (ttsConfig.edgeDialogueVoice || ttsConfig.edgeVoice) : ttsConfig.edgeVoice;
    if (ttsConfig.engine === "azure") return segment.dialogue ? (ttsConfig.azureDialogueVoice || ttsConfig.azureVoice) : ttsConfig.azureVoice;
    if (ttsConfig.engine === "microsoftTranslator") return segment.dialogue
      ? (ttsConfig.microsoftTranslatorDialogueVoice || ttsConfig.microsoftTranslatorVoice)
      : ttsConfig.microsoftTranslatorVoice;
    if (ttsConfig.engine === "googleCloud") return segment.dialogue
      ? (ttsConfig.googleCloudDialogueVoice || ttsConfig.googleCloudVoice)
      : ttsConfig.googleCloudVoice;
    if (ttsConfig.engine === "googleTranslate") return "ja";
    return "ja";
  }

  function synthesisFingerprint(): string {
    return JSON.stringify([
      ttsConfig.engine, ttsConfig.rate, ttsConfig.pitch,
      ttsConfig.voice, ttsConfig.dialogueVoice,
      ttsConfig.edgeVoice, ttsConfig.edgeDialogueVoice, ttsConfig.azureRegion,
      ttsConfig.azureVoice, ttsConfig.azureDialogueVoice, ttsConfig.microsoftTranslatorVoice,
      ttsConfig.microsoftTranslatorDialogueVoice, ttsConfig.googleCloudVoice,
      ttsConfig.googleCloudDialogueVoice,
      ttsConfig.providerOptions[ttsConfig.engine],
      ttsRuntime.azureKey, ttsRuntime.googleCloudKey,
      ttsConfig.customSources.find((source) => source.id === ttsConfig.engine),
      ttsRuntime.customKeys[ttsConfig.engine] ?? "",
    ]);
  }

  function synthesizedAudio(segmentIndex: number): Promise<string[]> {
    const segment = segments[segmentIndex];
    if (!segment) return Promise.reject(new Error("TTS segment is no longer available."));
    const fingerprint = synthesisFingerprint();
    const existing = audioCache.get(segmentIndex);
    if (existing?.fingerprint === fingerprint) return existing.promise;

    const engine = ttsConfig.engine;
    const synthesisTexts = splitTtsSynthesisText(segment.text, engine);
    const customSource = ttsConfig.customSources.find((source) => source.id === engine) ?? null;
    const customKey = ttsRuntime.customKeys[engine] ?? "";
    const voice = synthesisVoice(segment);
    const baseUrl = engine === "azure" ? ttsConfig.azureRegion : "";
    const apiKey = engine === "azure"
      ? ttsRuntime.azureKey
      : engine === "googleCloud"
        ? ttsRuntime.googleCloudKey
        : "";
    const rate = Number(customSource?.rate ?? ttsConfig.rate) || 1;
    const pitch = Number(ttsConfig.pitch) || 0;
    const options = ttsConfig.providerOptions[engine] ?? ttsConfig.providerOptions.edge;

    let promise: Promise<string[]>;
    promise = (async () => {
      const encodedParts: string[] = [];
      // These are provider payload chunks, not reader sentence boundaries.
      // Keep them ordered and let playback advance only after every part ends.
      for (const text of synthesisTexts) {
        encodedParts.push(await invoke<string>("tts_synthesize", {
          engine: engine === "device" && usesNativeWindowsSpeech ? "system" : engine,
          text,
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
      return encodedParts;
    })().catch((error) => {
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
    const segment = segments[segmentIndex];
    if (segment?.imagesAfter.length) {
      await onHoldImages(segment.chapterIndex, segment.sourceText, segment.imagesAfter);
      if (token !== runId || !active) return;
    }
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

  function speakSystemPart(playback: PendingSystemPlayback) {
    if (playback.token !== runId || !active) return;
    if (paused) {
      pendingSystemPlayback = playback;
      currentUtterance = null;
      return;
    }
    if (playback.partIndex >= playback.parts.length) {
      currentUtterance = null;
      pendingSystemPlayback = null;
      void advanceAfterSegment(playback.segmentIndex, playback.token);
      return;
    }

    const utterance = new SpeechSynthesisUtterance(playback.parts[playback.partIndex]);
    utterance.lang = "ja-JP";
    utterance.voice = playback.voice;
    utterance.rate = playback.rate;
    utterance.pitch = playback.pitch;
    utterance.volume = playback.volume;
    utterance.onend = () => {
      if (playback.token !== runId || !active) return;
      currentUtterance = null;
      const next = { ...playback, partIndex: playback.partIndex + 1 };
      if (paused) {
        pendingSystemPlayback = next;
        return;
      }
      speakSystemPart(next);
    };
    utterance.onerror = (event) => {
      if (playback.token !== runId || event.error === "canceled" || event.error === "interrupted") return;
      pendingSystemPlayback = null;
      errorMessage = "System TTS error: " + event.error;
      active = false;
      paused = false;
      sessionActive = false;
      onClearHighlight();
    };
    currentUtterance = utterance;
    window.speechSynthesis.speak(utterance);
    if (paused) window.speechSynthesis.pause();
  }

  function playAudioPart(playback: PendingAudioPlayback) {
    if (playback.token !== runId || !active) return;
    if (paused) {
      pendingAudioPlayback = playback;
      currentAudio = null;
      return;
    }
    if (playback.partIndex >= playback.encodedParts.length) {
      currentAudio = null;
      pendingAudioPlayback = null;
      void advanceAfterSegment(playback.segmentIndex, playback.token);
      return;
    }

    const audio = new Audio("data:" + playback.mimeType + ";base64," + playback.encodedParts[playback.partIndex]);
    audio.volume = Math.min(1, Math.max(0, Number(ttsConfig.volume) || 0));
    audio.playbackRate = Math.min(3, Math.max(0.5, Number(ttsConfig.playbackRate) || 1));
    audio.onended = () => {
      if (playback.token !== runId || !active) return;
      currentAudio = null;
      const next = { ...playback, partIndex: playback.partIndex + 1 };
      if (paused) {
        pendingAudioPlayback = next;
        return;
      }
      playAudioPart(next);
    };
    audio.onerror = () => {
      if (playback.token !== runId) return;
      pendingAudioPlayback = null;
      errorMessage = "The generated audio could not be played.";
      active = false;
      paused = false;
      sessionActive = false;
      onClearHighlight();
    };
    currentAudio = audio;
    if (playback.partIndex === 0) prefetchAhead(playback.segmentIndex);
    void audio.play().catch((error) => {
      if (playback.token !== runId) return;
      const isPauseAbort = paused && error instanceof Error && error.name === "AbortError";
      if (isPauseAbort) {
        currentAudio = null;
        pendingAudioPlayback = playback;
        return;
      }
      pendingAudioPlayback = null;
      errorMessage = String(error);
      active = false;
      paused = false;
      sessionActive = false;
      onClearHighlight();
    });
  }

  function cancelCurrent(): number {
    runId += 1;
    pendingNextPosition = null;
    pendingAudioPlayback = null;
    pendingSystemPlayback = null;
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

  async function playFrom(startIndex: number, startCharacterCount = 0) {
    const token = cancelCurrent();
    sessionStartIndex = startIndex;
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
      const targetCharacter = Math.max(0, Math.round(startCharacterCount));
      const startPosition = segments.findIndex((segment) =>
        segment.chapterIndex > startIndex ||
        (segment.chapterIndex === startIndex &&
          segment.readerCharacterStart + countReaderCharacters(segment.sourceText) > targetCharacter),
      );
      position = startPosition >= 0 ? startPosition : Math.max(0, segments.length - 1);
      sessionActive = true;
      active = true;
      await speakAt(position, token);
    } catch (error) {
      if (token === runId) {
        errorMessage = String(error);
        active = false;
        paused = false;
        sessionActive = false;
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

  function repeatCurrent() {
    if (!segments.length) return;
    jumpTo(Math.min(position, segments.length - 1));
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
      sessionActive = false;
      onClearHighlight();
      return;
    }
    const segment = segments[segmentIndex];
    await onReadSegment(segment.chapterIndex, segment.sourceText, segment.imagesBefore);
    if (token !== runId || !active) return;

    if (ttsConfig.engine === "device" && !usesNativeWindowsSpeech) {
      if (typeof window === "undefined" || !("speechSynthesis" in window)) {
        errorMessage = "System speech synthesis is not available in this WebView.";
        active = false;
        sessionActive = false;
        return;
      }
      const japaneseVoices = window.speechSynthesis.getVoices()
        .filter((voice) => voice.lang.trim().toLowerCase().startsWith("ja"));
      if (japaneseVoices.length === 0) {
        errorMessage = "No Japanese system speech voice is available in this WebView. Install the Japanese Windows Speech language feature, then refresh TTS voices. Narrator-only natural voices may not be exposed to WebView speech synthesis. Microsoft Edge (online) is an alternative.";
        active = false;
        paused = false;
        sessionActive = false;
        onClearHighlight();
        return;
      }
      const voiceUri = segment.dialogue && ttsConfig.dialogueVoice ? ttsConfig.dialogueVoice : ttsConfig.voice;
      const voice = japaneseVoices.find((item) => item.voiceURI === voiceUri)
        ?? japaneseVoices.find((item) => item.default)
        ?? japaneseVoices[0];
      speakSystemPart({
        segmentIndex,
        token,
        parts: splitTtsSynthesisText(segment.text, "device"),
        partIndex: 0,
        voice,
        rate: Math.min(3, Math.max(0.5, (Number(ttsConfig.rate) || 1) * (Number(ttsConfig.playbackRate) || 1))),
        pitch: Math.min(2, Math.max(0.5, Number(ttsConfig.pitch) || 1)),
        volume: Math.min(1, Math.max(0, Number(ttsConfig.volume) || 0)),
      });
      return;
    }

    try {
      const encodedParts = await synthesizedAudio(segmentIndex);
      if (token !== runId) return;
      const audioFormat = ttsConfig.engine === "device" && usesNativeWindowsSpeech
        ? "wav"
        : ttsConfig.engine.startsWith("custom:")
          ? (ttsConfig.customSources.find((source) => source.id === ttsConfig.engine)?.format ?? "mp3")
          : ["azure", "microsoftTranslator", "googleCloud"].includes(ttsConfig.engine)
            ? ttsConfig.providerOptions[ttsConfig.engine]?.format ?? "mp3"
            : "mp3";
      const mimeType = audioFormat === "wav" ? "audio/wav" : "audio/mpeg";
      const playback: PendingAudioPlayback = {
        segmentIndex,
        token,
        encodedParts,
        partIndex: 0,
        mimeType,
      };
      if (paused) {
        pendingAudioPlayback = playback;
        return;
      }
      playAudioPart(playback);
    } catch (error) {
      if (token === runId) {
        const isPauseAbort = paused && error instanceof Error && error.name === "AbortError";
        if (isPauseAbort) return;
        errorMessage = String(error);
        active = false;
        paused = false;
        sessionActive = false;
        onClearHighlight();
      }
    }
  }

  function pausePlayback() {
    paused = true;
    if (ttsConfig.engine === "device" && !usesNativeWindowsSpeech) {
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
    if (pendingSystemPlayback) {
      const pending = pendingSystemPlayback;
      pendingSystemPlayback = null;
      speakSystemPart(pending);
      return;
    }
    if (pendingAudioPlayback) {
      const pending = pendingAudioPlayback;
      pendingAudioPlayback = null;
      playAudioPart(pending);
      return;
    }
    if (ttsConfig.engine === "device" && !usesNativeWindowsSpeech) {
      if ("speechSynthesis" in window) window.speechSynthesis.resume();
    } else if (currentAudio) {
      try {
        await currentAudio.play();
      } catch (error) {
        if (paused && error instanceof Error && error.name === "AbortError") return;
        errorMessage = String(error);
        active = false;
        sessionActive = false;
        onClearHighlight();
      }
    }
  }

  async function togglePlayback() {
    autoPausedForLookup = false;
    if (!active) {
      await playFrom(currentChapterIndex, readerCharacterOffset());
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
    sessionActive = false;
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
      <p class="text-sm font-semibold">TTS</p>
      <p class="line-clamp-2 text-xs text-base-content/60">{title}</p>
      <p class="mt-1 text-xs text-base-content/60">
        {ttsConfig.engine === "device" ? "System voice" :
          ttsConfig.engine === "edge" ? "Microsoft Edge TTS" :
          ttsConfig.engine === "azure" ? "Microsoft Azure Speech" :
          ttsConfig.engine === "microsoftTranslator" ? "Microsoft Translator" :
          ttsConfig.engine === "googleCloud" ? "Google Cloud Text-to-Speech" :
          (ttsConfig.engine.startsWith("custom:")
            ? (ttsConfig.customSources.find((source) => source.id === ttsConfig.engine)?.name || "Custom online source")
            : "Google Translate")}
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

  <button class="btn btn-outline btn-sm" onclick={() => playFrom(currentChapterIndex, readerCharacterOffset())} disabled={loading}>
    Start at reader position
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
    <summary class="collapse-title min-h-0 py-3 text-sm font-medium">TTS settings</summary>
    <div class="collapse-content overflow-x-hidden">
      <TtsSettings compact={true} bookId={id} />
    </div>
  </details>
  <p class="text-xs text-base-content/50">The active sentence is highlighted in the book while it is being spoken.</p>
</div>
