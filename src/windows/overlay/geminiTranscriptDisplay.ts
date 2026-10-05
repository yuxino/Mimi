/** Display-only sentence breaks. Never feed this result into transcript assembly
 * or confirmed history: inserted whitespace would defeat cumulative matching. */
export function formatGeminiTranscriptForDisplay(text: string, minimumLineCharacters = 20): string {
  if (minimumLineCharacters <= 0) return text;
  const characters = Array.from(text);
  let rendered = "";
  let lineLength = 0;
  for (let index = 0; index < characters.length; index++) {
    const character = characters[index];
    rendered += character;
    lineLength = character === "\n" ? 0 : lineLength + 1;
    let afterClosers = index + 1;
    while (/^[”’"'」』）)】》]$/u.test(characters[afterClosers] ?? "")) afterClosers++;
    const asciiPeriod = character === "." &&
      /\s/u.test(characters[afterClosers] ?? "") &&
      !/\b(?:Mr|Mrs|Ms|Dr|Prof|vs|etc|[A-Z])\.$/iu.test(rendered) &&
      characters[index - 1] !== ".";
    if (!/[。！？!?]/u.test(character) && !asciiPeriod) continue;
    while (/^[。！？!?]$/u.test(characters[index + 1] ?? "")) {
      rendered += characters[++index];
      lineLength++;
    }
    while (/^[”’"'」』）)】》]$/u.test(characters[index + 1] ?? "")) {
      rendered += characters[++index];
      lineLength++;
    }
    let next = index + 1;
    while (characters[next] === " " || characters[next] === "\t") next++;
    if (lineLength >= minimumLineCharacters && next < characters.length && characters[next] !== "\n") {
      rendered += "\n";
      lineLength = 0;
      index = next - 1;
    }
  }
  return rendered;
}
