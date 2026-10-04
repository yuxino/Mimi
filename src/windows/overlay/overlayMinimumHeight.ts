import type { SettingsSnapshot } from "../../lib/types";

export const OVERLAY_BASE_MINIMUM_HEIGHT = 136;
const SERVICE_ROW_HEIGHT = 20;
export const OVERLAY_MAXIMUM_CHROME_HEIGHT = 85 + SERVICE_ROW_HEIGHT;
const METADATA_ROW_HEIGHT = 22;

/** Browser resize fallback for the core/overlay_layout.rs native rule.
 * Both implementations consume the shared layout contract in their tests. */
export function minimumOverlayHeight(settings: Pick<SettingsSnapshot,
  "audioInput" | "subtitleDisplayMode" | "targetLanguage" | "fontSize">
  & Partial<Pick<SettingsSnapshot, "showSubtitleTimestamps">>): number {
  // Keep a width-independent minimum so narrowing a short window cannot take
  // the extra service row out of its subtitle budget during a resize gesture.
  const minimumHeight = OVERLAY_BASE_MINIMUM_HEIGHT + SERVICE_ROW_HEIGHT;
  if (settings.audioInput !== "both") {
    return settings.showSubtitleTimestamps
      ? Math.ceil((minimumHeight + METADATA_ROW_HEIGHT) / 4) * 4
      : minimumHeight;
  }
  const font = Number.isFinite(settings.fontSize) ? Math.min(20, Math.max(14, settings.fontSize)) : 16;
  const translationLine = Math.ceil(font * 1.32);
  const rowHeight = settings.subtitleDisplayMode === "bilingual" && settings.targetLanguage !== "original"
    ? Math.ceil(Math.max(12, font * 0.82) * 1.32) + translationLine + 2 + 5
    : translationLine + 5;
  return Math.max(minimumHeight, Math.ceil((OVERLAY_MAXIMUM_CHROME_HEIGHT + 2 * (rowHeight + METADATA_ROW_HEIGHT)) / 4) * 4);
}
