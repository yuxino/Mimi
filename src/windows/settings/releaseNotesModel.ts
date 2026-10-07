import type { UiLanguage } from "../../lib/i18n";

export type ReleaseNotesLanguage = Exclude<UiLanguage, "system">;
export const RELEASE_NOTES_SOURCE_LIMIT = 32_000;
const DISPLAY_LIMIT = 8_000;

/** Select before bounding the visible body, so a long English section cannot
 * hide a later translation. Language headings inside code are ordinary text. */
export function selectLocalizedReleaseNotes(notes: string, language: ReleaseNotesLanguage): string {
  const source = notes.replace(/\r\n?/g, "\n").slice(0, RELEASE_NOTES_SOURCE_LIMIT).trim();
  const sections: Partial<Record<ReleaseNotesLanguage, string[]>> = {};
  const preamble: string[] = [];
  let current = preamble;
  let fence: string | undefined;
  for (const line of source.split("\n")) {
    const marker = line.trim().match(/^(`{3,}|~{3,})/);
    if (marker) {
      if (!fence) fence = marker[1];
      else if (marker[1][0] === fence[0] && marker[1].length >= fence.length) fence = undefined;
      current.push(line);
      continue;
    }
    const heading = !fence && line.match(/^ {0,3}##\s+(.+?)\s*#*\s*$/);
    const locale = heading ? headingLanguage(heading[1]) : undefined;
    if (locale) {
      current = sections[locale] ??= [];
    } else {
      current.push(line);
    }
  }
  const selected = [language, "en", "zh", "ja"]
    .map(locale => sections[locale as ReleaseNotesLanguage]?.join("\n").trim())
    .find(body => body);
  return (selected ?? preamble.join("\n")).trim().slice(0, DISPLAY_LIMIT);
}

function headingLanguage(heading: string): ReleaseNotesLanguage | undefined {
  switch (heading.toLowerCase()) {
    case "english": case "en": return "en";
    case "中文": case "简体中文": case "chinese": case "simplified chinese": case "zh": return "zh";
    case "繁體中文": case "繁体中文": case "traditional chinese": case "zh-tw": case "zh-hant": return "zh-TW";
    case "deutsch": case "german": case "de": return "de";
    case "français": case "french": case "fr": return "fr";
    case "한국어": case "korean": case "ko": return "ko";
    case "日本語": case "japanese": case "ja": return "ja";
    default: return undefined;
  }
}

export type ReleaseNotesBlock =
  | { kind: "heading" | "paragraph" | "code"; text: string }
  | { kind: "list"; ordered: boolean; items: string[] };

/** Intentionally small Markdown subset. No HTML, images or executable content. */
export function parseReleaseNotes(notes: string, language: ReleaseNotesLanguage): ReleaseNotesBlock[] {
  const lines = selectLocalizedReleaseNotes(notes, language).split("\n");
  const blocks: ReleaseNotesBlock[] = [];
  let paragraph: string[] = [];
  const flush = () => {
    if (paragraph.length) blocks.push({ kind: "paragraph", text: paragraph.join(" ") });
    paragraph = [];
  };
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index].trim();
    const fence = line.match(/^(`{3,}|~{3,})/);
    if (fence) {
      flush();
      const code: string[] = [];
      while (++index < lines.length) {
        const end = lines[index].trim().match(/^(`{3,}|~{3,})\s*$/);
        if (end && end[1][0] === fence[1][0] && end[1].length >= fence[1].length) break;
        code.push(lines[index]);
      }
      if (code.length) blocks.push({ kind: "code", text: code.join("\n") });
    } else if (!line || /^(?:-{3,}|\*{3,}|_{3,})$/.test(line)) {
      flush();
    } else if (/^#{1,6}\s/.test(line)) {
      flush();
      blocks.push({ kind: "heading", text: line.replace(/^#{1,6}\s+/, "").replace(/\s+#+$/, "") });
    } else {
      const item = line.match(/^([-+*]|\d+[.)])\s+(.+)$/);
      if (item) {
        flush();
        const ordered = /^\d/.test(item[1]);
        const previous = blocks.at(-1);
        if (previous?.kind === "list" && previous.ordered === ordered) previous.items.push(item[2]);
        else blocks.push({ kind: "list", ordered, items: [item[2]] });
      } else if (/^\s+/.test(lines[index]) && blocks.at(-1)?.kind === "list" && !paragraph.length) {
        const previous = blocks.at(-1) as Extract<ReleaseNotesBlock, { kind: "list" }>;
        previous.items[previous.items.length - 1] += ` ${line}`;
      } else {
        paragraph.push(line.replace(/^>\s?/, ""));
      }
    }
  }
  flush();
  return blocks;
}

export type ReleaseNotesInline =
  | { kind: "text" | "code"; text: string }
  | { kind: "strong" | "emphasis"; children: ReleaseNotesInline[] };

/** Link destinations are deliberately discarded. Mimi's only release opener is
 * a hard-coded native destination; release metadata cannot expand that access. */
export function parseReleaseNotesInline(text: string, depth = 0): ReleaseNotesInline[] {
  if (depth > 6) return [{ kind: "text", text }];
  const tokens: ReleaseNotesInline[] = [];
  let plain = "";
  const flush = () => {
    if (plain) tokens.push({ kind: "text", text: plain });
    plain = "";
  };
  for (let index = 0; index < text.length;) {
    if (text[index] === "\\" && /[\\`*_{}[\]()#+.!>-]/.test(text[index + 1] ?? "")) {
      plain += text[index + 1]; index += 2; continue;
    }
    const start = text.startsWith("![", index) ? index + 1 : index;
    if (text[start] === "[") {
      const labelEnd = text.indexOf("](", start + 1);
      if (labelEnd !== -1) {
        let balance = 1;
        let end = labelEnd + 2;
        for (; end < text.length && balance; end++) {
          if (text[end] === "\\") { end++; continue; }
          if (text[end] === "(") balance++;
          if (text[end] === ")") balance--;
        }
        if (!balance) {
          flush();
          tokens.push(...parseReleaseNotesInline(text.slice(start + 1, labelEnd), depth + 1));
          index = end; continue;
        }
      }
    }
    const delimiter = ["**", "__", "`", "*", "_"].find(value => text.startsWith(value, index));
    if (delimiter) {
      const end = text.indexOf(delimiter, index + delimiter.length);
      if (end > index + delimiter.length) {
        flush();
        const body = text.slice(index + delimiter.length, end);
        tokens.push(delimiter === "`"
          ? { kind: "code", text: body }
          : { kind: delimiter.length === 2 ? "strong" : "emphasis", children: parseReleaseNotesInline(body, depth + 1) });
        index = end + delimiter.length; continue;
      }
    }
    plain += text[index++];
  }
  flush();
  return tokens;
}
