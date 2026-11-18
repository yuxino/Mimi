import { I18N } from "../i18n";
import type { SettingsDraft, SettingsSnapshot, SubtitleDisplayMode } from "../types";

export const SUBTITLE_DISPLAY_OPTIONS: readonly { value: SubtitleDisplayMode; label: string; shortLabel: string }[] = [
  {
    value: "translation",
    get label() { return I18N.settings.displayTranslation; },
    get shortLabel() { return I18N.settings.transcriptTranslation; },
  },
  {
    value: "bilingual",
    get label() { return I18N.settings.displayBilingual; },
    get shortLabel() { return I18N.settings.displayBoth; },
  },
  {
    value: "original",
    get label() { return I18N.settings.displayOriginal; },
    get shortLabel() { return I18N.settings.transcriptSource; },
  },
];

/** Selectable range for the custom visible-subtitle count, and the stored
 * choice offered before the user picks one. */
export const MIN_VISIBLE_SUBTITLE_COUNT = 1;
export const MAX_VISIBLE_SUBTITLE_COUNT = 10;
export const DEFAULT_VISIBLE_SUBTITLE_COUNT = 3;

export const VISIBLE_SUBTITLE_COUNT_OPTIONS: readonly { value: string; label: string }[] = [
  { value: "all", get label() { return I18N.settings.visibleSubtitleCountAll; } },
  ...Array.from({ length: MAX_VISIBLE_SUBTITLE_COUNT - MIN_VISIBLE_SUBTITLE_COUNT + 1 }, (_, index) => {
    const count = MIN_VISIBLE_SUBTITLE_COUNT + index;
    return { value: String(count), get label() { return I18N.settings.visibleSubtitleCountRecent(count); } };
  }),
];

/** Clamps any stored or drafted count into the selectable 1..10 range. */
export function clampVisibleSubtitleCount(count: number | null | undefined): number {
  if (count === null || count === undefined || !Number.isFinite(count)) {
    return DEFAULT_VISIBLE_SUBTITLE_COUNT;
  }
  return Math.min(MAX_VISIBLE_SUBTITLE_COUNT, Math.max(MIN_VISIBLE_SUBTITLE_COUNT, Math.round(count)));
}

type VisibleSubtitleCountSettings = Pick<
  SettingsSnapshot,
  "customSubtitleCountEnabled" | "visibleSubtitleCount"
>;

/** The active display limit, or `null` when the custom count is off (the
 * default), which preserves the existing bounded overlay history. */
export function visibleSubtitleCountLimit(settings: VisibleSubtitleCountSettings): number | null {
  return settings.customSubtitleCountEnabled
    ? clampVisibleSubtitleCount(settings.visibleSubtitleCount)
    : null;
}

export function visibleSubtitleCountChoice(settings: VisibleSubtitleCountSettings): string {
  const limit = visibleSubtitleCountLimit(settings);
  return limit === null ? "all" : String(limit);
}

/** Submit a numeric choice atomically; "all" keeps the previously stored count. */
export function visibleSubtitleCountDraft(value: string): Pick<SettingsDraft, "customSubtitleCountEnabled" | "visibleSubtitleCount"> {
  return value === "all"
    ? { customSubtitleCountEnabled: false }
    : { customSubtitleCountEnabled: true, visibleSubtitleCount: clampVisibleSubtitleCount(Number(value)) };
}

/**
 * Display projection only: keeps the most recent confirmed subtitles when a
 * limit is active. The session history itself is untouched, so saved/exported
 * subtitles and the live tail are unaffected. Returns the original array
 * when no limit applies, preserving reference stability for memoization.
 */
export function limitRecentSubtitles<Entry>(
  history: readonly Entry[],
  settings: VisibleSubtitleCountSettings,
): Entry[] {
  const limit = visibleSubtitleCountLimit(settings);
  if (limit === null || history.length <= limit) return history as Entry[];
  return history.slice(history.length - limit);
}

export function subtitleDisplayShortcut(): string {
  return typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘⇧B" : "Ctrl+Shift+B";
}
