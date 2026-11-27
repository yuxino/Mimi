import { I18N } from "../i18n";
import type { SubtitleBackgroundColor, SubtitleColor, SubtitlePresetColor } from "../types";

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

export const DEFAULT_SUBTITLE_BACKGROUND_COLOR: SubtitleBackgroundColor = "#000000";

export const SUBTITLE_BACKGROUND_COLOR_OPTIONS: readonly { value: SubtitleBackgroundColor; label: string }[] = [
  { value: "#000000", get label() { return I18N.settings.backgroundColorBlack; } },
  { value: "#1C1C1E", get label() { return I18N.settings.backgroundColorCharcoal; } },
  { value: "#1E293B", get label() { return I18N.settings.backgroundColorSlate; } },
  { value: "#26211D", get label() { return I18N.settings.backgroundColorWarm; } },
  { value: "#FFFFFF", get label() { return I18N.settings.backgroundColorWhite; } },
];

/** Accept only six RGB digits; partial text input must never reach persistence. */
export function normalizeSubtitleHex(value: string): `#${string}` | null {
  const trimmed = value.trim();
  return /^#[0-9a-f]{6}$/i.test(trimmed) ? trimmed.toUpperCase() as `#${string}` : null;
}

/** Accept manual HEX input with or without a leading `#`. */
export function parseSubtitleHexInput(value: string): `#${string}` | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  return normalizeSubtitleHex(trimmed.startsWith("#") ? trimmed : `#${trimmed}`);
}

export function subtitleColorHex(color: SubtitleColor): string {
  return SUBTITLE_COLORS[color as SubtitlePresetColor] ?? normalizeSubtitleHex(color) ?? SUBTITLE_COLORS.white;
}

export function subtitleBackgroundHex(color?: SubtitleBackgroundColor): SubtitleBackgroundColor {
  return (color ? normalizeSubtitleHex(color) : null) ?? DEFAULT_SUBTITLE_BACKGROUND_COLOR;
}

function hexRgb(hex: string): [number, number, number] {
  return [
    Number.parseInt(hex.slice(1, 3), 16),
    Number.parseInt(hex.slice(3, 5), 16),
    Number.parseInt(hex.slice(5, 7), 16),
  ];
}

/** Background alpha never reduces subtitle text contrast. Older snapshots use 80% black. */
export function subtitleBackgroundColor(opacity = 80, color?: SubtitleBackgroundColor): string {
  const [red, green, blue] = hexRgb(subtitleBackgroundHex(color));
  return `rgba(${red},${green},${blue},${Math.min(100, Math.max(0, opacity)) / 100})`;
}

/** Whether a `#RRGGBB` color has high relative luminance (light ink / light surface). */
export function isLightHexColor(hex: string): boolean {
  const normalized = normalizeSubtitleHex(hex);
  if (!normalized) return false;
  const [red, green, blue] = hexRgb(normalized);
  return (0.2126 * red + 0.7152 * green + 0.0722 * blue) / 255 > 0.45;
}

/** Light, sufficiently opaque surfaces switch overlay chrome to dark ink for legibility. */
export function isLightSubtitleBackground(color?: SubtitleBackgroundColor, opacity = 80): boolean {
  if (Math.min(100, Math.max(0, opacity)) < 35) return false;
  return isLightHexColor(subtitleBackgroundHex(color));
}
