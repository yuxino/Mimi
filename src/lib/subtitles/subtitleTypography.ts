import { isLightHexColor, subtitleColorHex } from "./subtitleColor";
import { hexToRgba, type SubtitleColor, type SubtitleDisplayMode } from "../types";

export const MIN_SUBTITLE_FONT_SIZE = 14;
export const MAX_SUBTITLE_FONT_SIZE = 48;
export const DEFAULT_SUBTITLE_FONT_SIZE = 16;
export const SUBTITLE_LINE_HEIGHT = 1.32;
export const SUBTITLE_SOURCE_SCALE = 0.9;
export const IMMERSIVE_TEXT_SHADOW =
  "0 2px 5px rgba(0,0,0,0.98), 0 0 2px rgba(0,0,0,0.95), 0 0 12px rgba(0,0,0,0.72)";
export const LOW_OPACITY_DARK_TEXT_SHADOW =
  "0 1px 3px rgba(0,0,0,0.88), 0 0 2px rgba(0,0,0,0.75)";
export const LOW_OPACITY_LIGHT_TEXT_SHADOW =
  "0 1px 2px rgba(255,255,255,0.88), 0 0 2px rgba(255,255,255,0.72)";

export function normalizeSubtitleFontSize(value: number): number {
  return Number.isFinite(value)
    ? Math.min(MAX_SUBTITLE_FONT_SIZE, Math.max(MIN_SUBTITLE_FONT_SIZE, value))
    : DEFAULT_SUBTITLE_FONT_SIZE;
}

export function subtitleLaneFontSize(fontSize: number, displayMode: SubtitleDisplayMode, kind: "source" | "translation"): number {
  return kind === "source" && displayMode === "bilingual"
    ? Math.max(12, fontSize * SUBTITLE_SOURCE_SCALE)
    : fontSize;
}

export function subtitleLineHeight(fontSize: number): number {
  return Math.ceil(fontSize * SUBTITLE_LINE_HEIGHT);
}

interface SubtitleTextStyleOptions {
  fontSize: number;
  /** Resolved CSS family stack, reused while the selected font is unchanged. */
  fontFamily: string;
  displayMode: SubtitleDisplayMode;
  kind: "source" | "translation";
  color: SubtitleColor;
  blendsWithBackground?: boolean;
  tintReference?: boolean;
  keepTextOpaque?: boolean;
  preserveLineBreaks?: boolean;
  lightBackground?: boolean;
  /** Very translucent or transparent backgrounds add a contrasting halo so text stays legible. */
  lowOpacityBackground?: boolean;
}

/** The live lanes and settings sample share the same reading typography. */
export function subtitleTextStyle({
  fontSize, fontFamily, displayMode, kind, color, blendsWithBackground = false,
  tintReference = false, keepTextOpaque = false, preserveLineBreaks = false, lightBackground = false,
  lowOpacityBackground = false,
}: SubtitleTextStyleOptions) {
  const isReference = kind === "source" && displayMode === "bilingual";
  const laneFontSize = subtitleLaneFontSize(fontSize, displayMode, kind);
  const referenceNeutral = lightBackground ? "#141414" : "#FFFFFF";
  const resolvedHex = isReference && !tintReference ? referenceNeutral : subtitleColorHex(color);
  const textShadow = blendsWithBackground
    ? IMMERSIVE_TEXT_SHADOW
    : lowOpacityBackground
      ? isLightHexColor(resolvedHex) ? LOW_OPACITY_DARK_TEXT_SHADOW : LOW_OPACITY_LIGHT_TEXT_SHADOW
      : undefined;
  return {
    fontSize: laneFontSize,
    fontFamily,
    fontWeight: isReference ? 400 : 500,
    color: hexToRgba(resolvedHex, isReference && !keepTextOpaque ? 0.86 : 1),
    lineHeight: `${subtitleLineHeight(laneFontSize)}px`,
    overflowWrap: "break-word" as const,
    whiteSpace: preserveLineBreaks ? "pre-line" as const : undefined,
    textShadow,
  };
}
