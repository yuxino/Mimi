import type { SettingsSnapshot } from "../../../lib/types";
import {
  normalizeSubtitleFontSize, SUBTITLE_LINE_HEIGHT, SUBTITLE_SOURCE_SCALE,
} from "../../../lib/subtitles/subtitleTypography";

export const OVERLAY_BASE_MINIMUM_HEIGHT = 136;
const LEGACY_SERVICE_ROW_HEIGHT = 20;
export const OVERLAY_MAXIMUM_CHROME_HEIGHT = 85 + LEGACY_SERVICE_ROW_HEIGHT;
const METADATA_ROW_HEIGHT = 22;
const SOURCE_DIVIDER_HEIGHT = 15;

/** Browser resize fallback for the core/overlay_layout.rs native rule.
 * Both implementations consume the shared layout contract in their tests. */
export function minimumOverlayHeight(settings: Pick<SettingsSnapshot,
  "audioInput" | "subtitleDisplayMode" | "targetLanguage" | "fontSize">
  & Partial<Pick<SettingsSnapshot, "showSubtitleTimestamps">>): number {
  // Retain the existing resize minima, including historical service-row space.
  // The slimmer toolbar releases room inside the viewport; it must not shrink
  // the saved frame or the budget for large bilingual/source metadata.
  const sourceCount = settings.audioInput === "both" ? 2 : 1;
  const minimumHeight = OVERLAY_BASE_MINIMUM_HEIGHT + LEGACY_SERVICE_ROW_HEIGHT
    + (sourceCount === 1 && settings.showSubtitleTimestamps ? METADATA_ROW_HEIGHT : 0);
  const font = normalizeSubtitleFontSize(settings.fontSize);
  const translationLine = Math.ceil(font * SUBTITLE_LINE_HEIGHT);
  const rowHeight = settings.subtitleDisplayMode === "bilingual" && settings.targetLanguage !== "original"
    ? Math.ceil(Math.max(12, font * SUBTITLE_SOURCE_SCALE) * SUBTITLE_LINE_HEIGHT) + translationLine + 2 + 5
    : translationLine + 5;
  // Microphone identity can use its own row even with timestamps hidden.
  // Reserve the optional source divider so its toggle does not resize the frame.
  const metadataHeight = settings.audioInput !== "system" || settings.showSubtitleTimestamps ? METADATA_ROW_HEIGHT : 0;
  const required = OVERLAY_MAXIMUM_CHROME_HEIGHT + sourceCount * (rowHeight + metadataHeight)
    + (sourceCount - 1) * SOURCE_DIVIDER_HEIGHT;
  return Math.ceil(Math.max(minimumHeight, required) / 4) * 4;
}
