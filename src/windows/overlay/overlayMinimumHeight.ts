import type { SettingsSnapshot } from "../../lib/types";
import {
  normalizeSubtitleFontSize, SUBTITLE_LINE_HEIGHT, SUBTITLE_SOURCE_SCALE,
} from "../../lib/subtitleTypography";

export const OVERLAY_BASE_MINIMUM_HEIGHT = 136;
const SERVICE_ROW_HEIGHT = 20;
export const OVERLAY_MAXIMUM_CHROME_HEIGHT = 85 + SERVICE_ROW_HEIGHT;
const METADATA_ROW_HEIGHT = 22;
const SOURCE_DIVIDER_HEIGHT = 15;

/** Browser resize fallback for the core/overlay_layout.rs native rule.
 * Both implementations consume the shared layout contract in their tests. */
export function minimumOverlayHeight(settings: Pick<SettingsSnapshot,
  "audioInput" | "subtitleDisplayMode" | "targetLanguage" | "fontSize">
  & Partial<Pick<SettingsSnapshot, "showSubtitleTimestamps">>): number {
  // Keep a width-independent minimum so narrowing a short window cannot take
  // the extra service row out of its subtitle budget during a resize gesture.
  const sourceCount = settings.audioInput === "both" ? 2 : 1;
  const minimumHeight = OVERLAY_BASE_MINIMUM_HEIGHT + SERVICE_ROW_HEIGHT
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
