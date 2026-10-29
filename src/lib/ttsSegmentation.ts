export type TtsSegmentationRubyRange = { start: number; end: number; reading?: string };
export type TtsSentenceSpan = { start: number; end: number; dialogue: boolean };

const SINGLE_MATCHABLE_CHARACTERS = new Set(Array.from("○◯々〆〇〻ー"));

/** Exact counterpart of Android ReaderTextFilter.isReaderMatchableCodePoint(). */
export function isTtsReaderMatchableCharacter(character: string): boolean {
  const point = character.codePointAt(0);
  if (point === undefined) return false;
  return (point >= 0x30 && point <= 0x39) ||
    (point >= 0x41 && point <= 0x5a) ||
    (point >= 0x61 && point <= 0x7a) ||
    SINGLE_MATCHABLE_CHARACTERS.has(character) ||
    (point >= 0x3005 && point <= 0x3007) ||
    point === 0x303b ||
    (point >= 0x3041 && point <= 0x3096) ||
    (point >= 0x309d && point <= 0x309e) ||
    (point >= 0x30a1 && point <= 0x30fa) ||
    point === 0x30fc ||
    (point >= 0xff10 && point <= 0xff19) ||
    (point >= 0xff21 && point <= 0xff3a) ||
    (point >= 0xff41 && point <= 0xff5a) ||
    (point >= 0xff66 && point <= 0xff9d) ||
    (point >= 0xac00 && point <= 0xd7a3) ||
    (point >= 0x3131 && point <= 0x318e) ||
    (point >= 0x2e80 && point <= 0x2fdf) ||
    (point >= 0x3400 && point <= 0x4dbf) ||
    (point >= 0x4e00 && point <= 0x9fff) ||
    (point >= 0x20000 && point <= 0x2a6df) ||
    (point >= 0x2a700 && point <= 0x2b73f) ||
    (point >= 0x2b740 && point <= 0x2b81f) ||
    (point >= 0x2b820 && point <= 0x2ceaf) ||
    (point >= 0x2ceb0 && point <= 0x2ebef) ||
    (point >= 0x30000 && point <= 0x3134f) ||
    (point >= 0x31350 && point <= 0x323af);
}
const SENTENCE_CLOSERS = new Set(Array.from("」』）】\"’”！？!?。"));
const QUOTE_END_CLOSERS = new Set(Array.from("」』）】\"’”！？!?。"));

type Span = { start: number; end: number };
type QuoteRange = { start: number; end: number };

function hasReaderCharacter(value: string): boolean {
  return Array.from(value).some(isTtsReaderMatchableCharacter);
}

function characterOffsets(text: string): { codePointToUtf16: number[]; utf16ToCodePoint: number[] } {
  const codePointToUtf16 = [0];
  const utf16ToCodePoint = new Array<number>(text.length + 1);
  let utf16 = 0;
  let codePoint = 0;
  utf16ToCodePoint[0] = 0;

  for (const character of text) {
    utf16ToCodePoint[utf16] = codePoint;
    if (character.length === 2) utf16ToCodePoint[utf16 + 1] = codePoint;
    utf16 += character.length;
    codePoint++;
    codePointToUtf16.push(utf16);
    utf16ToCodePoint[utf16] = codePoint;
  }
  return { codePointToUtf16, utf16ToCodePoint };
}

function pairedJapaneseQuotes(text: string): QuoteRange[] {
  const stack: { opener: string; start: number }[] = [];
  const matched: QuoteRange[] = [];
  for (let index = 0; index < text.length; index++) {
    const character = text[index];
    if (character === "「" || character === "『") {
      stack.push({ opener: character, start: index });
      continue;
    }
    if (character !== "」" && character !== "』") continue;
    const opener = character === "」" ? "「" : "『";
    let found = -1;
    for (let stackIndex = stack.length - 1; stackIndex >= 0; stackIndex--) {
      if (stack[stackIndex].opener === opener) { found = stackIndex; break; }
    }
    if (found < 0) continue;
    while (stack.length - 1 > found) stack.pop();
    const paired = stack.pop();
    if (paired) matched.push({ start: paired.start, end: index + 1 });
  }

  matched.sort((a, b) => a.start - b.start || a.end - b.end);
  const merged: QuoteRange[] = [];
  for (const range of matched) {
    const previous = merged[merged.length - 1];
    if (previous && range.start < previous.end) previous.end = Math.max(previous.end, range.end);
    else merged.push({ ...range });
  }
  return merged;
}

function quoteBoundaries(text: string, quotes: QuoteRange[]): number[] {
  const boundaries = new Set<number>();
  for (const quote of quotes) {
    let end = quote.end;
    while (end < text.length && QUOTE_END_CLOSERS.has(text[end])) end++;
    boundaries.add(quote.start);
    boundaries.add(end);
  }
  return Array.from(boundaries).sort((a, b) => a - b);
}

/**
 * Mirrors Android TtsText's source-unit boundaries and TtsDialogue's chapter-wide
 * paired-quote cuts. Spans use UTF-16 source offsets; ruby/image offsets use code points.
 */
export function segmentTtsChapter(
  text: string,
  rubyRanges: TtsSegmentationRubyRange[] = [],
  imageOffsets: number[] = [],
): TtsSentenceSpan[] {
  if (!text) return [];
  const { codePointToUtf16, utf16ToCodePoint } = characterOffsets(text);
  const codePointCount = codePointToUtf16.length - 1;
  const utf16AtCodePoint = (offset: number) =>
    codePointToUtf16[Math.max(0, Math.min(codePointCount, Math.trunc(offset)))] ?? text.length;
  const codePointAtUtf16 = (offset: number) =>
    utf16ToCodePoint[Math.max(0, Math.min(text.length, offset))] ?? codePointCount;

  const validRubyRanges = rubyRanges.filter((range) =>
    Number.isInteger(range.start) && Number.isInteger(range.end) &&
    range.start >= 0 && range.end >= range.start && range.end <= codePointCount,
  );
  const utf16RubyRanges = validRubyRanges.map((range) => ({
    start: utf16AtCodePoint(range.start), end: utf16AtCodePoint(range.end),
    codePointStart: range.start, codePointEnd: range.end,
  }));
  const rubyByStart = new Map<number, { start: number; end: number; codePointStart: number; codePointEnd: number }>();
  const zeroWidthRuby = new Set<number>();
  for (const range of utf16RubyRanges) {
    if (range.start === range.end) zeroWidthRuby.add(range.start);
    else if (!rubyByStart.has(range.start)) rubyByStart.set(range.start, range);
  }

  const imageCuts = new Set<number>();
  for (const offset of imageOffsets) {
    if (!Number.isInteger(offset) || offset < 0 || offset > codePointCount) continue;
    const at = utf16AtCodePoint(offset);
    const rubyImage = zeroWidthRuby.has(at) ||
      utf16RubyRanges.some((range) => range.start < at && at < range.end);
    if (!rubyImage) imageCuts.add(at);
  }

  const sourceSentences: Span[] = [];
  let currentStart = -1;
  let sentenceEnded = false;
  const flush = (end: number) => {
    if (currentStart >= 0 && end > currentStart) {
      const candidate = text.slice(currentStart, end);
      if (hasReaderCharacter(candidate)) sourceSentences.push({ start: currentStart, end });
    }
    currentStart = -1;
    sentenceEnded = false;
  };

  let index = 0;
  while (index < text.length) {
    if (imageCuts.has(index)) flush(index);
    if (zeroWidthRuby.has(index) && sentenceEnded) flush(index);

    const ruby = rubyByStart.get(index);
    if (ruby) {
      if (sentenceEnded) flush(index);
      if (currentStart < 0) currentStart = index;
      index = ruby.end;
      continue;
    }

    const character = text[index];
    if (sentenceEnded && !SENTENCE_CLOSERS.has(character)) flush(index);
    if (character === "\n" || character === "\r") {
      flush(index);
      index++;
      continue;
    }
    if (currentStart >= 0 || character.trim().length > 0) {
      if (currentStart < 0) currentStart = index;
    }
    if ("。！？!?".includes(character)) sentenceEnded = true;
    index++;
  }
  flush(text.length);

  const quotes = pairedJapaneseQuotes(text);
  const boundaries = quoteBoundaries(text, quotes);
  const result: TtsSentenceSpan[] = [];
  for (const sentence of sourceSentences) {
    const cuts = boundaries.filter((boundary) => {
      if (boundary <= sentence.start || boundary >= sentence.end) return false;
      const codePoint = codePointAtUtf16(boundary);
      return !validRubyRanges.some((range) => codePoint > range.start && codePoint < range.end);
    });

    let start = sentence.start;
    const pieces: Span[] = [];
    for (const end of [...cuts, sentence.end]) {
      const candidate = text.slice(start, end);
      if (hasReaderCharacter(candidate)) {
        pieces.push({ start, end });
        start = end;
      } else if (end === sentence.end && pieces.length > 0) {
        pieces[pieces.length - 1].end = end;
      }
    }
    for (const piece of pieces) {
      result.push({
        ...piece,
        dialogue: quotes.some((quote) => quote.start < piece.end && quote.end > piece.start),
      });
    }
  }
  return result;
}

/** Provider payload chunks remain internal to one navigable sentence. */
export function splitTtsSynthesisText(text: string, engine: string): string[] {
  if (!text) return [text];
  if (engine === "device") {
    const result: string[] = [];
    let start = 0;
    const limit = 3999;
    while (start < text.length) {
      let end = Math.min(start + limit, text.length);
      if (end < text.length && text.charCodeAt(end - 1) >= 0xd800 && text.charCodeAt(end - 1) <= 0xdbff) end--;
      result.push(text.slice(start, end));
      start = end;
    }
    return result;
  }

  const limit = engine === "googleTranslate" ? 100 : 3000;
  const encoder = new TextEncoder();
  const result: string[] = [];
  let start = 0, cursor = 0, bytes = 0, boundary = 0;
  while (cursor < text.length) {
    const codePoint = text.codePointAt(cursor);
    if (codePoint === undefined) break;
    const character = String.fromCodePoint(codePoint);
    const size = encoder.encode(character).length;
    if (bytes + size > limit) {
      const end = boundary > start ? boundary : cursor;
      result.push(text.slice(start, end));
      start = end; cursor = end; bytes = 0; boundary = end;
      continue;
    }
    bytes += size;
    cursor += character.length;
    if ("。！？、.!? \n".includes(character)) boundary = cursor;
  }
  if (start < text.length) result.push(text.slice(start));
  return result.length ? result : [text];
}
