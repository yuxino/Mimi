import { I18N } from "./i18n";
import type { SubtitleColor, SubtitlePresetColor } from "./types";

// Same RGB values and display order as the Android subtitle settings.
export const SUBTITLE_COLORS: Record<SubtitlePresetColor, string> = {
  white: "#FFFFFF",
  teal: "#4FD1C5",
  yellow: "#FFD54F",
  green: "#9AE66E",
  pink: "#F49AB5",
};

export const SUBTITLE_COLOR_OPTIONS: readonly { value: SubtitlePresetColor; label: string }[] = [
  { value: "white", get label() { return I18N.settings.colorWhite; } },
  { value: "teal", get label() { return I18N.settings.colorTeal; } },
  { value: "yellow", get label() { return I18N.settings.colorYellow; } },
  { value: "green", get label() { return I18N.settings.colorGreen; } },
  { value: "pink", get label() { return I18N.settings.colorPink; } },
];

/** Accept only six RGB digits; partial text input must never reach persistence. */
export function normalizeSubtitleHex(value: string): `#${string}` | null {
  const trimmed = value.trim();
  return /^#[0-9a-f]{6}$/i.test(trimmed) ? trimmed.toUpperCase() as `#${string}` : null;
}

export function subtitleColorHex(color: SubtitleColor): string {
  return SUBTITLE_COLORS[color as SubtitlePresetColor] ?? normalizeSubtitleHex(color) ?? SUBTITLE_COLORS.white;
}

/** Background alpha never reduces subtitle text contrast. Older snapshots use 80%. */
export function subtitleBackgroundColor(opacity = 80): string {
  return `rgba(0,0,0,${Math.min(100, Math.max(0, opacity)) / 100})`;
}
