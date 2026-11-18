const ABBREVIATIONS = ["Mr", "Mrs", "Ms", "Dr", "Prof", "vs", "etc"];
const ABBREVIATION_END = new RegExp(`\\b(?:${ABBREVIATIONS.join("|")}|[A-Z])\\.$`, "iu");
// Include the period and one preceding character so truncation cannot invent
// a word boundary (for example, turning LongerProf. into the abbreviation).
const ABBREVIATION_TAIL_CHARACTERS = Math.max(...ABBREVIATIONS.map(value => value.length)) + 2;

function endsInAbbreviation(characters: readonly string[], periodIndex: number): boolean {
  let suffix = "";
  for (let index = Math.max(0, periodIndex + 1 - ABBREVIATION_TAIL_CHARACTERS); index <= periodIndex; index++) {
    suffix += characters[index];
  }
  return ABBREVIATION_END.test(suffix);
}

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
    let asciiPeriod = false;
    if (character === ".") {
      // Only a period needs lookahead. Re-scanning closing quotes at every
      // character makes a repeated-punctuation draft quadratic.
      let afterClosers = index + 1;
      while (/^[”’"'」』）)】》]$/u.test(characters[afterClosers] ?? "")) afterClosers++;
      asciiPeriod = /\s/u.test(characters[afterClosers] ?? "") &&
        !endsInAbbreviation(characters, index) &&
        characters[index - 1] !== ".";
    }
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
