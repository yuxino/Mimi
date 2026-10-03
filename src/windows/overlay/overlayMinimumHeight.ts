import type { SettingsSnapshot } from "../../lib/types";

export const OVERLAY_BASE_MINIMUM_HEIGHT = 136;
export const OVERLAY_MAXIMUM_CHROME_HEIGHT = 85;

/** Browser resize fallback for the core/overlay_layout.rs native rule.
 * Both implementations consume the shared layout contract in their tests. */
export function minimumOverlayHeight(settings: Pick<SettingsSnapshot,
  "audioInput" | "subtitleDisplayMode" | "targetLanguage" | "fontSize">): number {
  if (settings.audioInput !== "both") return OVERLAY_BASE_MINIMUM_HEIGHT;
  const font = Number.isFinite(settings.fontSize) ? Math.min(20, Math.max(14, settings.fontSize)) : 18;
  const translationLine = Math.ceil(font * 1.32);
  const rowHeight = settings.subtitleDisplayMode === "bilingual" && settings.targetLanguage !== "original"
    ? Math.ceil(Math.max(12, font * 0.82) * 1.32) + translationLine + 2 + 5
    : translationLine + 5;
  return Math.max(OVERLAY_BASE_MINIMUM_HEIGHT, Math.ceil((OVERLAY_MAXIMUM_CHROME_HEIGHT + 2 * rowHeight) / 4) * 4);
}
