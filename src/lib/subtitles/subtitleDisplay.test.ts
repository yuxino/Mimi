import { afterEach, describe, expect, it } from "vitest";
import { I18N, setStoredUiLanguage, UI_LANGUAGES } from "../i18n";
import {
  DEFAULT_VISIBLE_SUBTITLE_COUNT,
  VISIBLE_SUBTITLE_COUNT_OPTIONS,
  clampVisibleSubtitleCount,
  limitRecentSubtitles,
  visibleSubtitleCountChoice,
  visibleSubtitleCountDraft,
  visibleSubtitleCountLimit,
} from "./subtitleDisplay";

afterEach(() => setStoredUiLanguage("en"));

describe("visible subtitle count", () => {
  it("offers all subtitles and the recent counts 1 through 10", () => {
    expect(VISIBLE_SUBTITLE_COUNT_OPTIONS.map(option => option.value)).toEqual(
      ["all", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
    );
  });

  it("updates the existing option labels when the language changes", () => {
    for (const language of UI_LANGUAGES) {
      setStoredUiLanguage(language);
      expect(VISIBLE_SUBTITLE_COUNT_OPTIONS[0].label).toBe(I18N.settings.visibleSubtitleCountAll);
      for (let count = 1; count <= 10; count++) {
        expect(VISIBLE_SUBTITLE_COUNT_OPTIONS[count].label).toBe(I18N.settings.visibleSubtitleCountRecent(count));
      }
    }
  });

  it("maps a single choice to both stored fields without an intermediate enabled state", () => {
    expect(visibleSubtitleCountChoice({})).toBe("all");
    expect(visibleSubtitleCountChoice({ customSubtitleCountEnabled: false, visibleSubtitleCount: 7 })).toBe("all");
    expect(visibleSubtitleCountChoice({ customSubtitleCountEnabled: true, visibleSubtitleCount: 7 })).toBe("7");
    expect(visibleSubtitleCountDraft("all")).toEqual({ customSubtitleCountEnabled: false });
    expect(visibleSubtitleCountDraft("1")).toEqual({ customSubtitleCountEnabled: true, visibleSubtitleCount: 1 });
    expect(visibleSubtitleCountDraft("10")).toEqual({ customSubtitleCountEnabled: true, visibleSubtitleCount: 10 });
  });

  it("is off by default, so no limit applies", () => {
    expect(visibleSubtitleCountLimit({})).toBeNull();
    expect(visibleSubtitleCountLimit({ customSubtitleCountEnabled: false, visibleSubtitleCount: 2 })).toBeNull();
  });

  it("clamps stored counts into the selectable range", () => {
    expect(clampVisibleSubtitleCount(undefined)).toBe(DEFAULT_VISIBLE_SUBTITLE_COUNT);
    expect(clampVisibleSubtitleCount(0)).toBe(1);
    expect(clampVisibleSubtitleCount(11)).toBe(10);
    expect(clampVisibleSubtitleCount(5)).toBe(5);
    expect(visibleSubtitleCountLimit({ customSubtitleCountEnabled: true, visibleSubtitleCount: 99 })).toBe(10);
  });

  it("keeps only the most recent entries while enabled", () => {
    const history = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    expect(limitRecentSubtitles(history, { customSubtitleCountEnabled: true, visibleSubtitleCount: 1 })).toEqual([12]);
    expect(limitRecentSubtitles(history, { customSubtitleCountEnabled: true, visibleSubtitleCount: 10 }))
      .toEqual([3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    // The projection never mutates the underlying history.
    expect(history).toHaveLength(12);
  });

  it("returns the original history untouched while disabled", () => {
    const history = ["a", "b", "c"];
    expect(limitRecentSubtitles(history, { customSubtitleCountEnabled: false, visibleSubtitleCount: 1 })).toBe(history);
    expect(limitRecentSubtitles(history, {})).toBe(history);
    // A short history already within the limit keeps its identity too.
    expect(limitRecentSubtitles(history, { customSubtitleCountEnabled: true, visibleSubtitleCount: 10 })).toBe(history);
  });
});
