import { describe, expect, it } from "vitest";
import {
  isLightSubtitleBackground,
  normalizeSubtitleHex,
  parseSubtitleHexInput,
  subtitleBackgroundColor,
  subtitleBackgroundHex,
  subtitleColorHex,
} from "./subtitleColor";

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
  it("accepts manual HEX input with or without leading hash", () => {
    expect(parseSubtitleHexInput("1e293b")).toBe("#1E293B");
    expect(parseSubtitleHexInput(" #ff00aa ")).toBe("#FF00AA");
    expect(parseSubtitleHexInput("#fff")).toBeNull();
    expect(parseSubtitleHexInput("")).toBeNull();
  });
  it("resolves legacy presets and custom values through the same path", () => {
    expect(subtitleColorHex("yellow")).toBe("#FFD54F");
    expect(subtitleColorHex("#a1b2c3")).toBe("#A1B2C3");
  });
  it("formats overlay background color with alpha and detects light surfaces", () => {
    expect(subtitleBackgroundHex()).toBe("#000000");
    expect(subtitleBackgroundColor(80)).toBe("rgba(0,0,0,0.8)");
    expect(subtitleBackgroundColor(60, "#1e293b")).toBe("rgba(30,41,59,0.6)");
    expect(isLightSubtitleBackground("#000000", 80)).toBe(false);
    expect(isLightSubtitleBackground("#FFFFFF", 80)).toBe(true);
    expect(isLightSubtitleBackground("#FFFFFF", 20)).toBe(false);
  });
});
