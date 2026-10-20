import { describe, expect, it } from "vitest";
import type { AudioInput, SubtitleDisplayMode, TargetLanguage } from "../../lib/types";
import cases from "../../../shared/overlay-layout-contracts.json";
import { minimumOverlayHeight, OVERLAY_MAXIMUM_CHROME_HEIGHT } from "./overlayMinimumHeight";

describe("browser and native minimum-height contract", () => {
  it.each(cases)("keeps $audioInput $displayMode font $fontSize at $minimumHeight px", fixture => {
    expect(minimumOverlayHeight({ audioInput: fixture.audioInput as AudioInput,
      subtitleDisplayMode: fixture.displayMode as SubtitleDisplayMode,
      targetLanguage: fixture.targetLanguage as TargetLanguage, fontSize: fixture.fontSize,
      showSubtitleTimestamps: fixture.showSubtitleTimestamps }))
      .toBe(fixture.minimumHeight);
  });

  it("bounds malformed or unsupported browser font inputs", () => {
    const minimum = (fontSize: number) => minimumOverlayHeight({ audioInput: "both", subtitleDisplayMode: "bilingual", targetLanguage: "zh", fontSize });
    expect(minimum(-1)).toBe(minimum(14)); expect(minimum(100)).toBe(minimum(48));
    for (const font of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) expect(minimum(font)).toBe(minimum(16));
  });

  it("reserves the separate service row without taking height from a maximum-font bilingual body", () => {
    const settings = { audioInput: "system" as const, subtitleDisplayMode: "bilingual" as const,
      targetLanguage: "zh" as const, fontSize: 48 };
    expect(OVERLAY_MAXIMUM_CHROME_HEIGHT).toBe(61 + 20 + 12 + 10 + 2);
    // 43.2px source at 1.32 line height, 48px translation, gap and padding.
    expect(minimumOverlayHeight(settings) - OVERLAY_MAXIMUM_CHROME_HEIGHT).toBeGreaterThanOrEqual(58 + 64 + 2 + 5);
    expect(minimumOverlayHeight({ ...settings, showSubtitleTimestamps: true })).toBe(256);
    expect(minimumOverlayHeight({ ...settings, audioInput: "both" })).toBe(424);
  });

  it("keeps the existing compact frame for the saved default font", () => {
    const settings = { audioInput: "system" as const, subtitleDisplayMode: "bilingual" as const,
      targetLanguage: "zh" as const, fontSize: 16 };
    expect(minimumOverlayHeight(settings)).toBe(156);
    expect(minimumOverlayHeight({ ...settings, showSubtitleTimestamps: true })).toBe(180);
  });

  it("reserves microphone identity even when timestamps are hidden", () => {
    const settings = { audioInput: "microphone" as const, subtitleDisplayMode: "bilingual" as const,
      targetLanguage: "zh" as const, fontSize: 48 };
    expect(minimumOverlayHeight(settings)).toBe(256);
    expect(minimumOverlayHeight(settings) - OVERLAY_MAXIMUM_CHROME_HEIGHT).toBeGreaterThanOrEqual(22 + 58 + 64 + 2 + 5);
    expect(minimumOverlayHeight({ ...settings, fontSize: 16 })).toBe(176);
  });

  it("allocates two readable maximum-font bilingual tracks", () => {
    const settings = { audioInput: "both" as const, subtitleDisplayMode: "bilingual" as const,
      targetLanguage: "zh" as const, fontSize: 48 };
    const eachTrack = (minimumOverlayHeight(settings) - OVERLAY_MAXIMUM_CHROME_HEIGHT - 15) / 2;
    expect(eachTrack).toBeGreaterThanOrEqual(22 + 58 + 64 + 2 + 5);
    expect(minimumOverlayHeight({ ...settings, showSubtitleTimestamps: true })).toBe(minimumOverlayHeight(settings));
  });
});
