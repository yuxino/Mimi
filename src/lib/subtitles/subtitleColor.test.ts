import { describe, expect, it } from "vitest";
import { normalizeSubtitleHex, subtitleColorHex } from "./subtitleColor";

describe("custom subtitle colors", () => {
  it("normalizes complete RGB colors while preserving black and white", () => {
    expect(normalizeSubtitleHex(" #a1b2c3 ")).toBe("#A1B2C3");
    expect(normalizeSubtitleHex("#000000")).toBe("#000000");
    expect(normalizeSubtitleHex("#ffffff")).toBe("#FFFFFF");
  });
  it("rejects partial input, alpha, names and non-hex characters", () => {
    for (const value of ["", "#", "#abc", "123456", "#12345678", "#gg0011", "red", "url(x)", "#１２３４５６"]) {
      expect(normalizeSubtitleHex(value)).toBeNull();
    }
  });
  it("resolves legacy presets and custom values through the same path", () => {
    expect(subtitleColorHex("yellow")).toBe("#FFD54F");
    expect(subtitleColorHex("#a1b2c3")).toBe("#A1B2C3");
  });
});
