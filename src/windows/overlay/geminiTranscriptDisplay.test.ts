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
});
