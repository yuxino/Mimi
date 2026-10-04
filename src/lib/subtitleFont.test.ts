import { describe, expect, it } from "vitest";
import { normalizeSubtitleFontFamily, subtitleFontFamily } from "./subtitleFont";

describe("subtitle font family", () => {
  it.each([undefined, "", "　 "])("uses the language-aware UI fallback for a default or legacy value %s", value => {
    expect(subtitleFontFamily(value)).toBe("var(--mimi-ui-font)");
  });

  it("keeps an installed family as one quoted CSS name, including punctuation and keywords", () => {
    expect(subtitleFontFamily('  Example "Font", Family\\Name  '))
      .toBe('"Example \\"Font\\", Family\\\\Name", var(--mimi-ui-font)');
    expect(subtitleFontFamily("serif")).toBe('"serif", var(--mimi-ui-font)');
    expect(subtitleFontFamily('Font"; color: red; /*'))
      .toBe('"Font\\"; color: red; /*", var(--mimi-ui-font)');
  });

  it("preserves a saved missing font while retaining fallback for unavailable glyphs", () => {
    expect(normalizeSubtitleFontFamily("　未安装的字体　")).toBe("未安装的字体");
    expect(subtitleFontFamily("未安装的字体"))
      .toBe('"未安装的字体", var(--mimi-ui-font)');
  });

  it.each(["Font\0Name", "\nFont", "Font\r", "Font\tName", "Font\u007f", "Font\u0085"])(
    "falls back for control characters without interpreting the name: %j", value => {
      expect(normalizeSubtitleFontFamily(value)).toBe("");
      expect(subtitleFontFamily(value)).toBe("var(--mimi-ui-font)");
    },
  );

  it("bounds Unicode code points without splitting or truncating the family name", () => {
    const maximum = "𝔉".repeat(256);
    expect(normalizeSubtitleFontFamily(maximum)).toBe(maximum);
    expect(normalizeSubtitleFontFamily(`${maximum}𝔉`)).toBe("");
  });
});
