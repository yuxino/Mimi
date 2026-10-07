import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ReleaseNotes } from "./ReleaseNotes";
import { normalizeReleaseNotes } from "./softwareUpdateModel";
import { parseReleaseNotes, parseReleaseNotesInline, selectLocalizedReleaseNotes } from "./releaseNotesModel";

import release1512 from "../../../docs/releases/v1.5.12.md?raw";
import release1511 from "../../../docs/releases/v1.5.11.md?raw";
import release1510 from "../../../docs/releases/v1.5.10.md?raw";
import release159 from "../../../docs/releases/v1.5.9.md?raw";

const bilingual = "## English\n### Changes\n- **Fixed** startup.\n\n## 中文\n### 改进\n- **修复**启动问题。";

describe("localized release notes", () => {
  it.each([
    ["en", "### Changes\n- **Fixed** startup."],
    ["zh", "### 改进\n- **修复**启动问题。"],
    ["ja", "### Changes\n- **Fixed** startup."],
  ] as const)("selects %s content with English fallback", (language, expected) => {
    expect(selectLocalizedReleaseNotes(bilingual, language)).toBe(expected);
  });

  it.each([
    ["de", "Deutsch", "Startfehler behoben."],
    ["fr", "Français", "Problème de démarrage corrigé."],
    ["ko", "한국어", "시작 오류 수정."],
  ] as const)("selects %s notes when present and English otherwise", (language, heading, body) => {
    expect(selectLocalizedReleaseNotes(`${bilingual}\n## ${heading}\n${body}`, language)).toBe(body);
    expect(selectLocalizedReleaseNotes(bilingual, language)).toContain("Fixed");
  });

  it("uses Japanese notes when supplied and falls back from empty sections", () => {
    expect(selectLocalizedReleaseNotes(`${bilingual}\n## 日本語\n起動を修正。`, "ja")).toBe("起動を修正。");
    expect(selectLocalizedReleaseNotes(`${bilingual}\n## 日本語\n  `, "ja")).toContain("Fixed");
    expect(selectLocalizedReleaseNotes("## 中文\n修复。", "en")).toBe("修复。");
  });

  it("keeps an unsectioned release readable and ignores language headings inside code", () => {
    const unsectioned = "### Fixes\r\n\r\nA normal release.\r\n```md\r\n## 中文\r\n```";
    expect(selectLocalizedReleaseNotes(unsectioned, "zh")).toBe(unsectioned.replace(/\r\n/g, "\n"));
    expect(parseReleaseNotes(unsectioned, "en").at(-1)).toEqual({ kind: "code", text: "## 中文" });
  });

  it("selects a late translation before limiting the displayed body", () => {
    const notes = normalizeReleaseNotes(`## English\n${"Long. ".repeat(1_600)}\n## 中文\n完整的中文说明。`);
    expect(selectLocalizedReleaseNotes(notes, "zh")).toBe("完整的中文说明。");
    expect(selectLocalizedReleaseNotes(notes, "en")).toHaveLength(8_000);
    expect(selectLocalizedReleaseNotes("x".repeat(50_000), "ja")).toHaveLength(8_000);
  });
});

describe("safe release notes rendering", () => {
  it("renders headings, lists, emphasis, code and link labels without raw Markdown destinations", () => {
    const notes = `${bilingual}\n\n## 日本語\n### 修正\n- **起動**と \`mimi.exe\` を修正。\n- [リリース](https://github.com/yuxino/mimi/releases) を確認。\n\n手順は *簡単*。\n\n1. ダウンロード\n2. インストール`;
    const html = renderToStaticMarkup(createElement(ReleaseNotes, { notes, language: "ja" }));
    expect(html).toContain("<h4>修正</h4>");
    expect(html).toContain("<strong>起動</strong>");
    expect(html).toContain("<code>mimi.exe</code>");
    expect(html).toContain("<em>簡単</em>");
    expect(html).toContain("<ul>");
    expect(html).toContain("<ol>");
    expect(html).toContain("リリース を確認。");
    expect(html).not.toMatch(/https:|##|\*\*|href=/);
    expect(html).not.toContain("Fixed");
  });

  it("escapes HTML and never activates links, event handlers or remote images", () => {
    const notes = `<script>alert(1)</script>\n\n[run](javascript:alert(1)) [data](data:text/html,evil) [file](file:///private) ![image](https://example.invalid/tracker.png) [<img src=x onerror=evil>](https://example.invalid)`;
    const html = renderToStaticMarkup(createElement(ReleaseNotes, { notes, language: "en" }));
    expect(html).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(html).toContain("run data file image &lt;img");
    expect(html).not.toMatch(/<(?:script|img|a)\b|href=|javascript:|data:text|file:\/|tracker\.png/);
  });

  it("keeps malformed markup literal and accepts balanced URL parentheses without leftovers", () => {
    const html = renderToStaticMarkup(createElement(ReleaseNotes, {
      notes: "[broken](https://example.invalid\n\n**unfinished\n\n[valid](https://example.invalid/path_(part)) end",
      language: "en",
    }));
    expect(html).toContain("[broken](https://example.invalid");
    expect(html).toContain("**unfinished");
    expect(html).toContain("<p>valid end</p>");
    expect(parseReleaseNotesInline("\\*literal\\* and `**code**`")).toEqual([
      { kind: "text", text: "*literal* and " },
      { kind: "code", text: "**code**" },
    ]);
  });

  it("bounds pathological markup and code blocks without executing them", () => {
    const html = renderToStaticMarkup(createElement(ReleaseNotes, {
      notes: `[${"**".repeat(10_000)}](javascript:evil)`, language: "en",
    }));
    expect(html.length).toBeLessThan(50_000);
    const code = renderToStaticMarkup(createElement(ReleaseNotes, {
      notes: "```html\n<img src=x>\n**literal**", language: "en",
    }));
    expect(code).toContain("<pre><code>&lt;img src=x&gt;\n**literal**</code></pre>");
  });
});


describe("published bilingual release history", () => {
  it.each([
    ["v1.5.9", release159], ["v1.5.10", release1510],
    ["v1.5.11", release1511], ["v1.5.12", release1512],
  ])("preserves each complete language in %s and renders real Markdown", (_version, notes) => {
    const [english, chinese] = notes.replace("## English", "").split("## 中文").map(section => section.trim());
    expect(selectLocalizedReleaseNotes(normalizeReleaseNotes(notes), "en")).toBe(english);
    expect(selectLocalizedReleaseNotes(normalizeReleaseNotes(notes), "zh")).toBe(chinese);
    expect(selectLocalizedReleaseNotes(normalizeReleaseNotes(notes), "ja")).toBe(english);
    for (const language of ["zh", "en", "ja"] as const) {
      const html = renderToStaticMarkup(createElement(ReleaseNotes, { notes, language }));
      expect(html).toContain("<ul>");
      expect(html).toContain("<code>");
      expect(html).not.toMatch(/## English|## 中文|\*\*|https:\/\/github.com/);
    }
  });
});


it("selects distinct traditional and simplified Chinese release notes", () => {
  const notes = "## English\nEnglish changes\n## 中文\n简体更新\n## 繁體中文\n繁體更新";
  expect(selectLocalizedReleaseNotes(notes, "zh-TW")).toBe("繁體更新");
  expect(selectLocalizedReleaseNotes(notes, "zh")).toBe("简体更新");
  expect(selectLocalizedReleaseNotes("## English\nFallback\n## 中文\n简体", "zh-TW")).toBe("Fallback");
});
