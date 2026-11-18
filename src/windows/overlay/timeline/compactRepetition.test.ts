import { describe, expect, it } from "vitest";
import { compactRepetition } from "./compactRepetition";

describe("compact subtitle repetition presentation", () => {
  it("folds an extreme exact phrase run while retaining its surrounding text", () => {
    const unit = "Synthetic phrase, ";
    const original = `Before. ${unit.repeat(30)}After.`;

    expect(compactRepetition(original)).toBe("Before. Synthetic phrase, ×30 After.");
    expect(original).toBe(`Before. ${unit.repeat(30)}After.`);
  });

  it("handles a synthetic CJK phrase without copying the full run", () => {
    const original = `开头，${"合成词，".repeat(30)}结束。`;

    expect(compactRepetition(original)).toBe("开头，合成词， ×30结束。");
  });

  it.each([2, 3, 4, 5, 11])("retains an ordinary %i-fold repeat", (count) => {
    const original = "Synthetic repeated sentence, ".repeat(count);
    expect(compactRepetition(original)).toBe(original);
  });

  it("requires both twelve copies and at least 120 code points", () => {
    const tooShort = "短句，".repeat(12);
    expect(compactRepetition(tooShort)).toBe(tooShort);
    expect(compactRepetition("短句，".repeat(40))).toBe("短句， ×40");
    expect(compactRepetition("안녕하세요".repeat(24))).toBe("안녕하세요 ×24");
  });

  it("retains long repeated sentences whose period exceeds 32 code points", () => {
    const sentence = "An ordinary long sentence with different meaningful words. ";
    const original = sentence.repeat(20);
    expect(compactRepetition(original)).toBe(original);
  });

  it("accepts an exact 32-code-point phrase and leaves a 33-code-point period intact", () => {
    const exact = "abcdefghijklmnopqrstuvwxyz012345";
    const tooLong = `${exact}6`;
    expect(Array.from(exact)).toHaveLength(32);
    expect(compactRepetition(exact.repeat(12))).toBe(`${exact} ×12`);
    expect(compactRepetition(tooLong.repeat(12))).toBe(tooLong.repeat(12));
  });

  it("does not normalize similar wording, punctuation, case, or whitespace", () => {
    const original = [
      ...Array.from({ length: 20 }, (_, index) => `Synthetic phrase ${index}, `),
      "Same phrase, ".repeat(6),
      "Same phrase， ".repeat(6),
      "same phrase, ".repeat(6),
      "Same phrase,  ".repeat(6),
    ].join("");
    expect(compactRepetition(original)).toBe(original);
  });

  it("counts astral Unicode code points and retains complete emoji units", () => {
    expect(compactRepetition("🙂".repeat(119))).toBe("🙂".repeat(119));
    expect(compactRepetition("🙂".repeat(120))).toBe("🙂 ×120");
    expect(compactRepetition("👩🏽‍🚀".repeat(30))).toBe("👩🏽‍🚀 ×30");
    expect(compactRepetition("啊".repeat(120))).toBe("啊 ×120");
  });

  it("retains a partial unit after the exact whole run", () => {
    const original = `${"안녕하세요".repeat(24)}안녕`;
    expect(compactRepetition(original)).toBe("안녕하세요 ×24안녕");
  });

  it("folds independent runs in one text without joining them", () => {
    const original = `${"First phrase, ".repeat(20)}Break. ${"第二个短语，".repeat(24)}End.`;
    expect(compactRepetition(original)).toBe("First phrase, ×20 Break. 第二个短语， ×24End.");
  });

  it("keeps an identical delimiter from the preceding text outside the repeated phrase", () => {
    expect(compactRepetition(`Prefix,${"hello, ".repeat(20)}Suffix`)).toBe("Prefix,hello, ×20 Suffix");
    expect(compactRepetition("「短语」".repeat(30))).toBe("「短语」 ×30");
  });

  it("retains a partial prefix and uses the normally ordered delimited phrase", () => {
    expect(compactRepetition(`词，${"合成词，".repeat(30)}`)).toBe("词，合成词， ×30");
    expect(compactRepetition(`词，${"合成词，".repeat(30)}合成`)).toBe("词，合成词， ×30合成");
  });

  it("retains whitespace-only text", () => {
    const original = " \t\n".repeat(100);
    expect(compactRepetition(original)).toBe(original);
  });

  it("handles a 64 KiB periodic input in one bounded projection", () => {
    const original = "abcdefg ".repeat(8_192);
    expect(original.length).toBe(65_536);
    expect(compactRepetition(original)).toBe("abcdefg ×8192 ");
  });

  it("retains 64 KiB of overlapping multilingual phrases with differing Unicode text", () => {
    const encoder = new TextEncoder();
    const pieces: string[] = [];
    let bytes = 0;
    for (let index = 0; ; index += 1) {
      const phrase = `中한🙂${index.toString(36)} `;
      const phraseBytes = encoder.encode(phrase).length;
      if (bytes + phraseBytes > 65_536) break;
      pieces.push(phrase);
      bytes += phraseBytes;
    }
    const original = pieces.join("") + "x".repeat(65_536 - bytes);
    expect(encoder.encode(original)).toHaveLength(65_536);
    expect(compactRepetition(original)).toBe(original);
  });
});
