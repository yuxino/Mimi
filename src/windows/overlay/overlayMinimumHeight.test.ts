import { describe, expect, it } from "vitest";
import type { AudioInput, SubtitleDisplayMode, TargetLanguage } from "../../lib/types";
import cases from "../../../shared/overlay-layout-contracts.json";
import { minimumOverlayHeight } from "./overlayMinimumHeight";

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
    expect(minimum(-1)).toBe(minimum(14)); expect(minimum(100)).toBe(minimum(20));
    for (const font of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) expect(minimum(font)).toBe(minimum(16));
  });
});
