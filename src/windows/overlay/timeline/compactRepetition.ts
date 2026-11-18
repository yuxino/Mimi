const MIN_REPEAT_COUNT = 12;
const MIN_REPEAT_CODE_POINTS = 120;
const MAX_PHRASE_CODE_POINTS = 32;
const SEPARATOR = /^[\s\p{P}]$/u;
const WHITESPACE = /^\s$/u;

/**
 * A compact-display projection only. The caller retains the unmodified caption
 * for reading, accessibility, translation, and opted-in session history.
 *
 * Each period (1–32 Unicode code points) gets one backwards comparison pass.
 * A match length gives the exact number of whole repeated units at that offset.
 * This is O(32n) time and O(n) extra space, including adversarial 64 KiB captions;
 * it never searches each suffix again or uses a repetition backtracking regex.
 */
export function compactRepetition(text: string): string {
  if (text.length < MIN_REPEAT_CODE_POINTS) return text;
  const points = Array.from(text);
  const length = points.length;
  if (length < MIN_REPEAT_CODE_POINTS) return text;

  const separatorCounts = new Uint32Array(length + 1);
  const nonWhitespaceCounts = new Uint32Array(length + 1);
  for (let index = 0; index < length; index += 1) {
    separatorCounts[index + 1] = separatorCounts[index] + Number(SEPARATOR.test(points[index]));
    nonWhitespaceCounts[index + 1] = nonWhitespaceCounts[index] + Number(!WHITESPACE.test(points[index]));
  }

  const periods = new Uint8Array(length);
  const counts = new Uint32Array(length);
  const separated = new Uint8Array(length);
  for (let period = 1; period <= MAX_PHRASE_CODE_POINTS; period += 1) {
    let matching = 0;
    for (let start = length - period - 1; start >= 0; start -= 1) {
      matching = points[start] === points[start + period] ? matching + 1 : 0;
      const count = 1 + Math.floor(matching / period);
      const span = count * period;
      if (count < MIN_REPEAT_COUNT || span < MIN_REPEAT_CODE_POINTS
        || nonWhitespaceCounts[start + period] === nonWhitespaceCounts[start]) continue;

      const hasSeparator = Number(separatorCounts[start + period] > separatorCounts[start]);
      // A delimited phrase must end at a separator and start at a real text
      // boundary. Otherwise a partial prefix could rotate the same period
      // into a representative with its words in the wrong order. Quoted or
      // bracketed units at those boundaries still qualify.
      const endsWithSeparator = separatorCounts[start + period] > separatorCounts[start + period - 1];
      const startsAtBoundary = start === 0 || separatorCounts[start] > separatorCounts[start - 1];
      if (hasSeparator && (!endsWithSeparator || !startsAtBoundary)) continue;
      const previousSpan = periods[start] * counts[start];
      // Prefer an actual delimited phrase and its shortest exact period, rather
      // than displaying several copies as one larger invented representative.
      if (hasSeparator > separated[start]
        || (hasSeparator === separated[start]
          && (span > previousSpan || (span === previousSpan && period < periods[start])))) {
        periods[start] = period;
        counts[start] = count;
        separated[start] = hasSeparator;
      }
    }
  }

  const pieces: string[] = [];
  let copyFrom = 0;
  for (let start = 0; start < length;) {
    const period = periods[start];
    if (period === 0) {
      start += 1;
      continue;
    }
    pieces.push(points.slice(copyFrom, start).join(""));
    const phrase = points.slice(start, start + period).join("");
    const trailingWhitespace = phrase.match(/\s+$/u)?.[0] ?? "";
    pieces.push(`${phrase.trimEnd()} ×${counts[start]}${trailingWhitespace}`);
    start += period * counts[start];
    copyFrom = start;
  }
  if (copyFrom === 0) return text;
  pieces.push(points.slice(copyFrom).join(""));
  return pieces.join("");
}
