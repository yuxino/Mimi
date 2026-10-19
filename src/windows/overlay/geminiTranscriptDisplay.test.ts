import { describe, expect, it } from "vitest";
import { formatGeminiTranscriptForDisplay as format } from "./geminiTranscriptDisplay";

describe("Gemini display-only sentence breaks", () => {
  it("groups short replies and breaks a long CJK line with closing punctuation", () => {
    const raw = "好的。嗯。然后这是一段足够长的合成字幕用于显示测试。「到这里结束！？」下一段开始。";
    expect(format(raw)).toBe("好的。嗯。然后这是一段足够长的合成字幕用于显示测试。\n「到这里结束！？」下一段开始。");
    expect(raw).not.toContain("\n");
    expect(format("简短的合成字幕「结束！？」继续。", 4)).toBe("简短的合成字幕「结束！？」\n继续。");
  });

  it("counts Unicode characters and preserves existing newlines", () => {
    expect(format("😀😀合成。下一段。", 5)).toBe("😀😀合成。\n下一段。");
    expect(format("第一段。\n简短。后续", 10)).toBe("第一段。\n简短。后续");
    expect(format("原始文本。继续", 0)).toBe("原始文本。继续");
  });

  it("leaves decimals, domains, abbreviations and subword spelling intact", () => {
    const raw = "Dr. Smith uses 3.14 at example.test. Next synthetic sentence!  A final fragment";
    expect(format(raw)).toBe("Dr. Smith uses 3.14 at example.test.\nNext synthetic sentence!\nA final fragment");
    expect(format('This is a long synthetic "sentence." Next.', 20)).toBe('This is a long synthetic "sentence."\nNext.');
    expect(format("translation is very very good.")).toBe("translation is very very good.");
  });

  it.each(["Mr", "Mrs", "Ms", "Dr", "Prof", "vs", "etc", "A", "pRoF", "ſ", "K"])(
    "keeps the full abbreviation %s with its preceding Unicode word-boundary context",
    abbreviation => {
      for (const prefix of ["", " ", "(", "中", "😀", "é", "a\u0301"]) {
        const raw = `${prefix}${abbreviation}. Next`;
        expect(format(raw, 1), raw).toBe(raw);
      }
      for (const prefix of ["x", "_", "9", "ſ", "K"]) {
        const raw = `${prefix}${abbreviation}. Next`;
        expect(format(raw, 1), raw).toBe(`${prefix}${abbreviation}.\nNext`);
      }
    },
  );

  it("preserves abbreviation boundaries around repeated periods, punctuation and closing quotes", () => {
    expect(format("Prof.. Next", 1)).toBe("Prof.. Next");
    expect(format("Prof.”』 Next", 1)).toBe("Prof.”』 Next");
    expect(format("LongerProf.”』 Next", 1)).toBe("LongerProf.”』\nNext");
    expect(format("Prof.!? Next", 1)).toBe("Prof.!?\nNext");
    expect(format("Prof.\nNext", 1)).toBe("Prof.\nNext");
  });

  it("bounds abbreviation lookbehind work across a maximum-size ASCII draft", () => {
    const raw = "A. ".repeat(Math.floor(64 * 1024 / 3));
    const abbreviationPattern = /\b(?:Mr|Mrs|Ms|Dr|Prof|vs|etc|[A-Z])\.$/iu.source;
    const test = RegExp.prototype.test;
    let checkedCharacters = 0;
    let checks = 0;
    try {
      RegExp.prototype.test = function (value: string) {
        if (this.source === abbreviationPattern) {
          checkedCharacters += value.length;
          checks++;
        }
        return test.call(this, value);
      };
      expect(format(raw)).toBe(raw);
    } finally {
      RegExp.prototype.test = test;
    }
    expect(checks).toBe(Math.floor(raw.length / 3));
    expect(checkedCharacters).toBeLessThanOrEqual(raw.length * 3);
  });

  it.each([false, true])("scans long runs of Unicode closing punctuation at most linearly (after period=%s)", afterPeriod => {
    const closers = '”’"\'」』）)】》'.repeat(800);
    const raw = afterPeriod ? `Synthetic sentence.${closers}\tNext 😀` : closers;
    const expected = afterPeriod ? `Synthetic sentence.${closers}\nNext 😀` : closers;
    const closerPattern = /^[”’"'」』）)】》]$/u.source;
    const test = RegExp.prototype.test;
    let comparisons = 0;
    try {
      RegExp.prototype.test = function (value: string) {
        if (this.source === closerPattern && ++comparisons > raw.length * 3) {
          throw new Error("Repeated closing punctuation exceeded its linear scan budget");
        }
        return test.call(this, value);
      };
      expect(format(raw)).toBe(expected);
    } finally {
      RegExp.prototype.test = test;
    }
    expect(comparisons).toBeLessThanOrEqual(raw.length * 3);
  });
});
