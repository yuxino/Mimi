import { describe, expect, it } from "vitest";
import {
  type ServiceProvider,
  type SubtitleSnapshot,
} from "../../lib/types";
import {
  buildSubtitleBlocks,
  subtitleLaneBudget,
  computeActivityPhaseFromSignals,
  hasSubtitleContent,
  pendingSourceTranslation,
  usesAtomicSubtitlePreview,
  visibleLiveSubtitle,
  visibleLiveSubtitles,
} from "./overlayModel";

describe("atomic subtitle preview capability", () => {
  // These families match the current Rust TranslationClient factory, including
  // custom ASR's independent text destination and Alibaba's text alternatives.
  it.each<[ServiceProvider, boolean]>([
    ["alibabaCloud", true], ["deepLX", true],
    ["customDashScopeASR", true], ["customOpenAIASR", true], ["appleSpeech", true],
    ["openAIRealtime", false], ["googleGeminiLive", false], ["azureOpenAIRealtime", false],
    ["volcanoEngine", false], ["tencentCloud", false], ["baiduTranslate", false], ["xAIRealtime", false],
  ])("uses complete preview pairs for %s only when its route supports them", (provider, expected) => {
    expect(usesAtomicSubtitlePreview(provider)).toBe(expected);
  });

  it("keeps missing and unrecognized saved-case routes out of atomic projection", () => {
    for (const provider of [undefined, null, "unknown", "constructor", { provider: "alibabaCloud" }]) {
      expect(usesAtomicSubtitlePreview(provider)).toBe(false);
    }
  });
});


describe("same-text committed subtitles", () => {
  const pair = { source: "Mimi", translation: "Mimi", createdAt: 1 };

  it("keeps the translation when the original lane is hidden", () => {
    expect(buildSubtitleBlocks([pair], "translation")).toEqual([
      {
        id: "history-1",
        createdAt: 1,
        presentation: "latestCommitted",
        source: null,
        translation: "Mimi",
      },
    ]);
  });

  it.each(["bilingual", "original"] as const)("keeps one original lane in %s mode", (mode) => {
    expect(buildSubtitleBlocks([pair], mode)).toEqual([
      {
        id: "history-1",
        createdAt: 1,
        presentation: "latestCommitted",
        source: "Mimi",
        translation: null,
      },
    ]);
  });
});

describe("live presentation epochs", () => {
  const first = { source: "First original.", translation: "第一句译文。", createdAt: 1 };
  const second = { source: "Second original.", translation: "第二句译文。", createdAt: 2 };
  const tail = { source: "Live source.", translation: "实时译文。", isStreaming: true };
  const liveId = (history: SubtitleSnapshot["history"], mode: "original" | "translation" | "bilingual" = "bilingual") => buildSubtitleBlocks(history, mode, tail).at(-1)!.id;

  it("changes a live layout key on confirmation without matching draft wording to a final", () => {
    expect(liveId([first, second])).not.toBe(liveId([first]));
    expect(liveId([first])).not.toBe(liveId([]));
  });
  it("keeps the epoch stable across modes, history eviction, and draft revisions", () => {
    for (const mode of ["original", "translation", "bilingual"] as const) {
      expect(liveId([second], mode)).toBe(liveId([first, second]));
      expect(buildSubtitleBlocks([second], mode, { ...tail, source: "Corrected live source." }).at(-1)!.id).toBe(liveId([second]));
    }
    expect(liveId([])).toBe("live"); // Clear removes the Timeline; the next empty-history mount starts fresh.
  });
  it("keeps an actual live owner through an earlier confirmation and changes it only for another owner", () => {
    const ownedB = { ...tail, utteranceId: "synthetic-epoch:1:B" };
    for (const mode of ["original", "translation", "bilingual"] as const) {
      const id = buildSubtitleBlocks([first], mode, ownedB).at(-1)!.id;
      expect(buildSubtitleBlocks([first, second], mode, ownedB).at(-1)!.id).toBe(id);
      expect(buildSubtitleBlocks([second], mode, { ...ownedB, source: "Corrected owner B." }).at(-1)!.id).toBe(id);
      expect(buildSubtitleBlocks([second], mode, { ...ownedB, utteranceId: "synthetic-epoch:1:C" }).at(-1)!.id).not.toBe(id);
      expect(buildSubtitleBlocks([], mode, { ...ownedB, utteranceId: "synthetic-epoch:2:B" }).at(-1)!.id).not.toBe(id);
    }
  });
});

const settings = {
  sourceLanguage: "auto" as const,
  targetLanguage: "zh" as const,
};

function subtitles(
  source: SubtitleSnapshot["source"],
  translation: SubtitleSnapshot["translation"] = {
    text: "",
    isFinal: false,
  },
  history: SubtitleSnapshot["history"] = [],
): SubtitleSnapshot {
  return { source, translation, history };
}

describe("live subtitle display mode", () => {
  it("prefers a translation draft over source recognition", () => {
    expect(
      visibleLiveSubtitle(
        subtitles(
          { text: "source draft", isFinal: false },
          { text: "译文草稿", isFinal: false },
        ),
        settings,
        "en",
        false,
        false,
      ),
    ).toEqual({ text: "译文草稿", isFinal: false, kind: "translation" });
  });

  it("does not flash source recognition before the first translation", () => {
    expect(
      visibleLiveSubtitle(
        subtitles({ text: "still recognizing", isFinal: false }),
        settings,
        "en",
        false,
        false,
      ),
    ).toBeNull();
  });

  it("does not replace missing translation with source after timeout", () => {
    expect(
      visibleLiveSubtitle(
        subtitles(
          { text: "recognized final", isFinal: true },
          { text: "", isFinal: true },
          [
            {
              source: "previous source",
              translation: "上一句译文",
              createdAt: 1,
            },
          ],
        ),
        settings,
        "en",
        false,
        true,
      ),
    ).toBeNull();
  });

  it("treats same-language recognition as final subtitle text", () => {
    expect(
      visibleLiveSubtitle(
        subtitles({ text: "中文识别结果", isFinal: true }),
        settings,
        "zh",
        false,
        false,
      ),
    ).toEqual({ text: "中文识别结果", isFinal: true, kind: "source" });
  });

  it("does not duplicate a source final already committed to history", () => {
    const history = [
      {
        source: "finished source",
        translation: "完成的译文",
        createdAt: 1,
      },
    ];
    expect(
      visibleLiveSubtitle(
        subtitles(
          { text: "finished source", isFinal: true },
          { text: "完成的译文", isFinal: true },
          history,
        ),
        settings,
        "en",
        false,
        false,
      ),
    ).toBeNull();
  });

  it("does not append a repeated source while the next translation is pending", () => {
    const history = [
      {
        source: "repeated lyric",
        translation: "重复歌词",
        createdAt: 1,
      },
    ];
    expect(
      visibleLiveSubtitle(
        subtitles(
          { text: "repeated lyric", isFinal: true },
          { text: "重复歌词", isFinal: true },
          history,
        ),
        settings,
        "en",
        true,
        false,
      ),
    ).toBeNull();
  });

  it("does not append a repeated source after its translation times out", () => {
    const history = [
      {
        source: "repeated lyric",
        translation: "上一遍歌词",
        createdAt: 1,
      },
    ];
    expect(
      visibleLiveSubtitle(
        subtitles(
          { text: "repeated lyric", isFinal: true },
          { text: "上一遍歌词", isFinal: true },
          history,
        ),
        settings,
        "en",
        false,
        true,
      ),
    ).toBeNull();
  });

  it("keeps recognition visible in explicitly selected original mode", () => {
    expect(
      visibleLiveSubtitle(
        subtitles({ text: "original draft", isFinal: false }),
        { ...settings, targetLanguage: "original" },
        "en",
        false,
        false,
      ),
    ).toEqual({ text: "original draft", isFinal: false, kind: "source" });
  });

  it("waits for translation while automatic source language is unknown", () => {
    expect(
      visibleLiveSubtitle(
        subtitles({ text: "unclassified draft", isFinal: false }),
        settings,
        null,
        true,
        false,
      ),
    ).toBeNull();
  });
});

describe("activity phase signals", () => {
  const base = {
    statusKind: "listening" as const,
    isPaused: false,
    detectedLanguage: "ja",
    isTranslationPending: false,
    hasRecognizingSourceDraft: false,
  };

  it("distinguishes recognizing and translating without subtitle text", () => {
    expect(
      computeActivityPhaseFromSignals(
        { ...base, hasRecognizingSourceDraft: true },
        settings,
      ),
    ).toBe("recognizing");
    expect(
      computeActivityPhaseFromSignals(
        { ...base, isTranslationPending: true },
        settings,
      ),
    ).toBe("translating");
  });

  it("keeps pause and lifecycle transitions ahead of stream activity", () => {
    expect(
      computeActivityPhaseFromSignals(
        { ...base, isPaused: true, isTranslationPending: true },
        settings,
      ),
    ).toBe("paused");
    expect(
      computeActivityPhaseFromSignals(
        { ...base, statusKind: "stopping", hasRecognizingSourceDraft: true },
        settings,
      ),
    ).toBe("connecting");
  });

  it.each(["error", "idle"] as const)(
    "never reports %s as listening even when stream or pause flags remain",
    (statusKind) => {
      expect(
        computeActivityPhaseFromSignals(
          {
            ...base,
            statusKind,
            isPaused: true,
            isTranslationPending: true,
            hasRecognizingSourceDraft: true,
          },
          settings,
        ),
      ).toBe(statusKind);
    },
  );
});


describe("compact lane budget", () => {
  it("keeps the ordinary display-mode limits before the viewport has been measured", () => {
    expect(subtitleLaneBudget("bilingual", true)).toEqual({ source: 1, translation: 2 });
    expect(subtitleLaneBudget("bilingual", false)).toEqual({ source: 2, translation: 0 });
    expect(subtitleLaneBudget("translation", true)).toEqual({ source: 0, translation: 2 });
    expect(subtitleLaneBudget("original", false)).toEqual({ source: 2, translation: 0 });
  });

  it("allocates whole lines in a short viewport without removing either bilingual lane", () => {
    expect(subtitleLaneBudget("bilingual", true, 49, 20)).toEqual({ source: 1, translation: 1 });
    expect(subtitleLaneBudget("bilingual", true, 120, 20)).toEqual({ source: 2, translation: 2 });
    expect(subtitleLaneBudget("translation", true, 40, 20)).toEqual({ source: 0, translation: 1 });
    expect(subtitleLaneBudget("original", false, 40, 20)).toEqual({ source: 1, translation: 0 });
    expect(subtitleLaneBudget("original", false, 48, 20)).toEqual({ source: 1, translation: 0 });
    expect(subtitleLaneBudget("bilingual", false, 48, 20)).toEqual({ source: 2, translation: 0 });
  });

  it("balances long original and translation lanes at the measured native reading height", () => {
    for (const sourceScale of [0.88, 0.9]) {
      const pair = subtitleLaneBudget("bilingual", true, 143, 17,
        { source: 420, translation: 460 }, sourceScale);
      expect(pair).toEqual({ source: 3, translation: 3 });
      const sourceLine = Math.ceil(17 * sourceScale * 1.32);
      expect(pair.source * sourceLine + pair.translation * 23).toBeLessThanOrEqual(143);
    }
    expect(subtitleLaneBudget("bilingual", true, 49, 17,
      { source: 420, translation: 460 })).toEqual({ source: 1, translation: 1 });
  });

  it("preserves short-lane borrowing at the same native height", () => {
    expect(subtitleLaneBudget("bilingual", true, 143, 17,
      { source: 20, translation: 690 }, 0.88)).toEqual({ source: 1, translation: 5 });
    expect(subtitleLaneBudget("bilingual", true, 143, 17,
      { source: 660, translation: 23 }, 0.88)).toEqual({ source: 6, translation: 1 });
  });

  it("uses a tall window instead of clipping every language to two lines", () => {
    expect(subtitleLaneBudget("original", false, 240, 18).source).toBe(10);
    expect(subtitleLaneBudget("translation", true, 240, 18).translation).toBe(10);
    const pair = subtitleLaneBudget("bilingual", true, 240, 18);
    expect(pair.source).toBeGreaterThan(2);
    expect(pair.translation).toBeGreaterThan(2);
    expect(pair.source * 22 + pair.translation * 24).toBeLessThanOrEqual(240);
  });

  it("gives a short lane's unused space to the longer language in either direction", () => {
    expect(subtitleLaneBudget("bilingual", true, 240, 18, { source: 22, translation: 480 }))
      .toEqual({ source: 1, translation: 9 });
    expect(subtitleLaneBudget("bilingual", true, 240, 18, { source: 440, translation: 24 }))
      .toEqual({ source: 9, translation: 1 });
  });
});

describe("subtitle display preference", () => {
  const pair = { source: "Hello world", translation: "你好世界", createdAt: 1 };

  it("preserves translation-only history and selects original without changing the target", () => {
    expect(buildSubtitleBlocks([pair], "translation")).toEqual([
      { id: "history-1", createdAt: 1, presentation: "latestCommitted", source: null, translation: "你好世界" },
    ]);
    expect(buildSubtitleBlocks([pair], "original")[0]).toMatchObject({
      source: "Hello world",
      translation: null,
    });
    expect(visibleLiveSubtitle(
      subtitles({ text: "New source", isFinal: false }, { text: "旧译文", isFinal: false }),
      { ...settings, subtitleDisplayMode: "original" }, "en", true, false,
    )).toEqual({ text: "New source", isFinal: false, kind: "source" });
  });

  it("groups each committed utterance into one block with a single timestamp", () => {
    const blocks = buildSubtitleBlocks(
      [pair, { ...pair, source: "Next", translation: "下一句", createdAt: 2 }],
      "bilingual",
    );
    expect(blocks.map((block) => [block.source, block.translation])).toEqual([
      ["Hello world", "你好世界"],
      ["Next", "下一句"],
    ]);
    expect(blocks.map((block) => block.createdAt)).toEqual([1, 2]);
    expect(blocks.map((block) => block.presentation)).toEqual(["history", "latestCommitted"]);
  });

  it("marks the newest committed block until a live tail replaces the compact slot", () => {
    const tail = { source: "Streaming", translation: null, isStreaming: true };
    const blocks = buildSubtitleBlocks([pair], "bilingual", tail);
    expect(blocks.map((block) => block.presentation)).toEqual(["history", "live"]);
    expect(blocks[1]).toEqual({
      id: "live-after-history-1",
      createdAt: null,
      presentation: "live",
      source: "Streaming",
      translation: null,
      streaming: true,
    });
    // An empty tail is not a block: the newest committed utterance keeps the
    // compact presentation instead.
    expect(buildSubtitleBlocks([pair], "bilingual", { source: null, translation: null, isStreaming: false }))
      .toEqual(buildSubtitleBlocks([pair], "bilingual"));
  });

  it("does not duplicate same-language or original-target history", () => {
    const same = { ...pair, translation: pair.source };
    expect(buildSubtitleBlocks([same], "bilingual")[0]).toMatchObject({
      source: pair.source,
      translation: null,
    });
  });

  it("keeps sources with empty translations and never substitutes translations for missing originals", () => {
    expect(buildSubtitleBlocks([{ ...pair, translation: "" }], "bilingual")[0]).toMatchObject({
      source: pair.source,
      translation: null,
    });
    expect(buildSubtitleBlocks([{ ...pair, source: "" }], "original")).toEqual([]);
  });

  it.each([
    [true, false],
    [false, true],
    [false, false],
  ])("never pairs new recognition with a stale translation (pending %s, timeout %s)", (pending, timedOut) => {
    const snapshot = subtitles(
      { text: "Next source", isFinal: true },
      { text: pair.translation, isFinal: true },
      [pair],
    );
    expect(visibleLiveSubtitle(snapshot, { ...settings, subtitleDisplayMode: "bilingual" }, "en", pending, timedOut))
      .toEqual({ text: "Next source", isFinal: true, kind: "source" });
  });

  it.each(["original", "bilingual"] as const)("removes the %s preview once its pair is committed", (subtitleDisplayMode) => {
    expect(visibleLiveSubtitle(
      subtitles({ text: pair.source, isFinal: true }, { text: pair.translation, isFinal: true }, [pair]),
      { ...settings, subtitleDisplayMode }, "en", false, false,
    )).toBeNull();
    expect(visibleLiveSubtitle(
      subtitles({ text: "", isFinal: false }),
      { ...settings, subtitleDisplayMode }, "en", false, false,
    )).toBeNull();
  });
});


describe("asynchronous bilingual stream arrival", () => {
  it.each([false, true])("keeps a translation without source visible (final %s)", (isFinal) => {
    expect(visibleLiveSubtitle(
      subtitles({ text: "", isFinal: false }, { text: "Available translation", isFinal }),
      { ...settings, subtitleDisplayMode: "bilingual" }, "en", false, false,
    )).toEqual({ text: "Available translation", isFinal, kind: "translation" });
  });

  it.each(["bilingual", "original"] as const)("does not repeat the committed source when the next translation arrives first in %s mode", (subtitleDisplayMode) => {
    expect(visibleLiveSubtitle(
      subtitles(
        { text: "Previous source", isFinal: true },
        { text: "下一句译文", isFinal: false },
        [{ source: "Previous source", translation: "上一句译文", createdAt: 1 }],
      ),
      { ...settings, subtitleDisplayMode }, "en", false, false,
    )).toBeNull();
  });

  it("does not hide an actual repeated source while its translation is pending", () => {
    expect(visibleLiveSubtitle(
      subtitles(
        { text: "Repeated lyric", isFinal: true },
        { text: "重复歌词", isFinal: true },
        [{ source: "Repeated lyric", translation: "重复歌词", createdAt: 1 }],
      ),
      { ...settings, subtitleDisplayMode: "bilingual" }, "en", true, false,
    )).toEqual({ text: "Repeated lyric", isFinal: true, kind: "source" });
  });
});

describe("bilingual preview rows", () => {
  const confirmed = { source: "Completed synthetic source A.", translation: "已确认的合成译文 A。", createdAt: 1 };

  it.each([
    { pending: true, timedOut: false },
    { pending: false, timedOut: true },
    { pending: false, timedOut: false },
  ])("does not reopen an atomic-route confirmed source during pending=$pending timeout=$timedOut", ({ pending, timedOut }) => {
    const snapshot = { ...subtitles({ text: confirmed.source, isFinal: true },
      { text: confirmed.translation, isFinal: true }, [confirmed]), previewPair: null };
    for (const subtitleDisplayMode of ["original", "bilingual"] as const) {
      const previews = visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode }, "en", pending, timedOut, true);
      expect(previews).toEqual([]);
      expect(buildSubtitleBlocks(snapshot.history, subtitleDisplayMode)).toHaveLength(1);
    }
  });

  it.each(["original", "bilingual"] as const)("keeps a new same-text draft, different final, and identified source on the atomic %s route", subtitleDisplayMode => {
    for (const source of [
      { text: confirmed.source, isFinal: false },
      { text: "Different synthetic source B.", isFinal: true },
      { text: confirmed.source, isFinal: true, utteranceId: "explicit-new-source" },
    ]) {
      const snapshot = { ...subtitles(source, { text: confirmed.translation, isFinal: true }, [confirmed]), previewPair: null };
      expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode }, "en", true, false, true))
        .toEqual([{ ...source, kind: "source" }]);
    }
  });

  it("does not deduplicate a new completed preview or a non-atomic repeated final by matching its text", () => {
    const snapshot = { ...subtitles({ text: confirmed.source, isFinal: true },
      { text: confirmed.translation, isFinal: true }, [confirmed]),
      previewPair: { source: confirmed.source, translation: confirmed.translation } };
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode: "bilingual" }, "en", true, false, true))
      .toEqual([
        { text: confirmed.source, isFinal: false, kind: "source", isStable: true },
        { text: confirmed.translation, isFinal: false, kind: "translation", isStable: true },
      ]);
    expect(visibleLiveSubtitles({ ...snapshot, previewPair: null }, { ...settings, subtitleDisplayMode: "bilingual" }, "en", true, false, false))
      .toEqual([{ text: confirmed.source, isFinal: true, kind: "source" }]);
  });

  it("uses the completed pair's owner while recognition has already advanced to another source", () => {
    const snapshot = { ...subtitles({ text: "Latest raw source C.", isFinal: false, utteranceId: "synthetic-owner-C" }),
      previewPair: { source: "Completed preview B.", translation: "完整合成预览 B。", utteranceId: "synthetic-owner-B" } };
    for (const subtitleDisplayMode of ["bilingual", "translation"] as const) {
      const previews = visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode }, "en", true, false, true);
      expect(previews.every(preview => preview.utteranceId === "synthetic-owner-B")).toBe(true);
      expect(previews.map(preview => preview.text)).toEqual(subtitleDisplayMode === "bilingual"
        ? [snapshot.previewPair.source, snapshot.previewPair.translation] : [snapshot.previewPair.translation]);
    }
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode: "original" }, "en", true, false, true))
      .toEqual([{ text: "Latest raw source C.", isFinal: false, kind: "source", utteranceId: "synthetic-owner-C" }]);
  });

  it.each(["Synthetic paired text.", "  Synthetic paired text.\n"])("keeps one atomic bilingual lane when the translation is %j", translation => {
    const pair = { source: "Synthetic paired text.", translation, utteranceId: "synthetic-pair-owner" };
    const snapshot = { ...subtitles({ text: "Newer raw recognition.", isFinal: false, utteranceId: "synthetic-raw-owner" }),
      previewPair: pair };
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode: "bilingual" }, "en", true, false, true))
      .toEqual([{ text: pair.source, isFinal: false, kind: "source", isStable: true, utteranceId: pair.utteranceId }]);
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode: "translation" }, "en", true, false, true))
      .toEqual([{ text: pair.translation, isFinal: false, kind: "translation", isStable: true, utteranceId: pair.utteranceId }]);
  });

  it("previews the original and its streaming translation together", () => {
    expect(visibleLiveSubtitles(
      subtitles(
        { text: "We next bring our cautery device.", isFinal: false },
        { text: "接下来，我们使用电凝设备。", isFinal: false },
      ),
      { ...settings, subtitleDisplayMode: "bilingual" }, "en", true, false,
    )).toEqual([
      { text: "We next bring our cautery device.", isFinal: false, kind: "source" },
      { text: "接下来，我们使用电凝设备。", isFinal: false, kind: "translation" },
    ]);
  });

  it("keeps one preview row outside bilingual mode", () => {
    const snapshot = subtitles(
      { text: "We next bring our cautery device.", isFinal: false },
      { text: "接下来，我们使用电凝设备。", isFinal: false },
    );
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode: "translation" }, "en", true, false))
      .toEqual([{ text: "接下来，我们使用电凝设备。", isFinal: false, kind: "translation" }]);
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode: "original" }, "en", true, false))
      .toEqual([{ text: "We next bring our cautery device.", isFinal: false, kind: "source" }]);
  });

  it("drops a committed pair from the previews and keeps the next translation", () => {
    const committed = { source: "Previous source", translation: "上一句译文", createdAt: 1 };
    expect(visibleLiveSubtitles(
      subtitles({ text: "Previous source", isFinal: true }, { text: "上一句译文", isFinal: true }, [committed]),
      { ...settings, subtitleDisplayMode: "bilingual" }, "en", false, false,
    )).toEqual([]);
    expect(visibleLiveSubtitles(
      subtitles({ text: "Previous source", isFinal: true }, { text: "下一句译文", isFinal: false }, [committed]),
      { ...settings, subtitleDisplayMode: "bilingual" }, "en", false, false,
    )).toEqual([{ text: "下一句译文", isFinal: false, kind: "translation" }]);
  });

  it("never stacks the same recognition and translation text twice", () => {
    expect(visibleLiveSubtitles(
      subtitles(
        { text: "今日は晴れです。", isFinal: false },
        { text: "今日は晴れです。", isFinal: false },
      ),
      { sourceLanguage: "ja", targetLanguage: "original", subtitleDisplayMode: "bilingual" }, "ja", false, false,
    )).toEqual([{ text: "今日は晴れです。", isFinal: false, kind: "source" }]);
  });

  it.each([
    { sourceLanguage: "ja" as const, targetLanguage: "original" as const, detectedLanguage: "ja" },
    { sourceLanguage: "auto" as const, targetLanguage: "ja" as const, detectedLanguage: "ja" },
  ])("keeps one language when same-language drafts briefly differ", ({ sourceLanguage, targetLanguage, detectedLanguage }) => {
    expect(visibleLiveSubtitles(
      subtitles(
        { text: "今日は晴れ", isFinal: false },
        { text: "今日は晴れです。", isFinal: false },
      ),
      { sourceLanguage, targetLanguage, subtitleDisplayMode: "bilingual" }, detectedLanguage, false, false,
    )).toEqual([{ text: "今日は晴れ", isFinal: false, kind: "source" }]);
  });

  it("stacks previews only while both lines belong to the same utterance", () => {
    const streaming = (sourceUtterance: string | null, translationUtterance: string | null) =>
      subtitles(
        { text: "Next sentence", isFinal: false, utteranceId: sourceUtterance },
        { text: "下一句", isFinal: false, utteranceId: translationUtterance },
      );
    const bilingual = { ...settings, subtitleDisplayMode: "bilingual" } as const;

    expect(visibleLiveSubtitles(streaming("item_a", "item_a"), bilingual, "en", false, false)).toEqual([
      { text: "Next sentence", isFinal: false, kind: "source", utteranceId: "item_a" },
      { text: "下一句", isFinal: false, kind: "translation", utteranceId: "item_a" },
    ]);
    // The translation still answers the previous sentence: the original stays
    // alone instead of pairing the two streams by arrival order.
    expect(visibleLiveSubtitles(streaming("item_b", "item_a"), bilingual, "en", false, false)).toEqual([
      { text: "Next sentence", isFinal: false, kind: "source", utteranceId: "item_b" },
    ]);
    // A translation whose utterance is unknown must not stack either: it can
    // still be the previous sentence's text.
    expect(visibleLiveSubtitles(streaming("item_b", null), bilingual, "en", false, false)).toEqual([
      { text: "Next sentence", isFinal: false, kind: "source", utteranceId: "item_b" },
    ]);
    // Providers without utterance identity keep stacking both streams.
    expect(visibleLiveSubtitles(streaming(null, null), bilingual, "en", false, false)).toEqual([
      { text: "Next sentence", isFinal: false, kind: "source" },
      { text: "下一句", isFinal: false, kind: "translation" },
    ]);
  });
});


it("reports translation in one input while the other input already matches the target language", () => {
  const tracks = ["zh", "en"].map((detectedLanguage, index) => ({
    audioSource: index ? "microphone" as const : "system" as const,
    source: { text: "", isFinal: false }, translation: { text: "", isFinal: false }, history: [],
    detectedLanguage, isTranslationPending: true, isTranslationTimedOut: false,
  }));
  const sourceTranslationPending = pendingSourceTranslation({ ...subtitles({ text: "", isFinal: false }), tracks }, settings);
  expect(sourceTranslationPending).toBe(true);
  expect(computeActivityPhaseFromSignals({ statusKind: "listening", isPaused: false, detectedLanguage: "zh",
    isTranslationPending: true, sourceTranslationPending, hasRecognizingSourceDraft: false }, settings)).toBe("translating");
});


describe("current complete display pair", () => {
  const pair = {
    source: "Complete synthetic source A.",
    translation: "完整的合成译文 A。",
    utteranceId: "synthetic-display-A",
  };
  const bilingual = { ...settings, subtitleDisplayMode: "bilingual" } as const;
  const pairRows = [
    { text: pair.source, isFinal: false, kind: "source", isStable: true, utteranceId: pair.utteranceId },
    { text: pair.translation, isFinal: false, kind: "translation", isStable: true, utteranceId: pair.utteranceId },
  ];

  it.each([false, true])("keeps the complete pair without retained history on atomic=%s routes", atomic => {
    const snapshot = { ...subtitles({ text: "", isFinal: false }), displayPair: pair };
    expect(visibleLiveSubtitles(snapshot, bilingual, "en", false, false, atomic)).toEqual(pairRows);
    expect(snapshot.history).toEqual([]);
    expect(hasSubtitleContent(snapshot)).toBe(true);
  });

  it.each([
    { pending: true, timedOut: false },
    { pending: false, timedOut: true },
    { pending: false, timedOut: false },
  ])("preserves the complete owner's lanes while the next recognition advances (pending=$pending timeout=$timedOut)", ({ pending, timedOut }) => {
    const snapshot = {
      ...subtitles(
        { text: "New synthetic source B.", isFinal: false, utteranceId: "synthetic-raw-B" },
        { text: "未完成", isFinal: false, utteranceId: "synthetic-raw-B" },
      ),
      displayPair: pair,
      previewPair: null,
    };
    expect(visibleLiveSubtitles(snapshot, bilingual, "en", pending, timedOut, true)).toEqual(pairRows);
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode: "translation" }, "en", pending, timedOut, true))
      .toEqual([pairRows[1]]);
    // Original mode follows the current recognition owner independently.
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode: "original" }, "en", pending, timedOut, true))
      .toEqual([{ text: "New synthetic source B.", isFinal: false, kind: "source", utteranceId: "synthetic-raw-B" }]);
  });

  it.each(["bilingual", "translation"] as const)("does not append the current pair twice when it is the latest %s history entry", subtitleDisplayMode => {
    const snapshot = {
      ...subtitles({ text: "", isFinal: false }, { text: "", isFinal: false }, [{ ...pair, createdAt: 1 }]),
      displayPair: pair,
      previewPair: null,
    };
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode }, "en", false, false, true)).toEqual([]);
    expect(buildSubtitleBlocks(snapshot.history, subtitleDisplayMode)).toHaveLength(1);
  });

  it("keeps a new complete preview even when an earlier utterance has identical text", () => {
    const repeatedPair = { ...pair, utteranceId: "synthetic-display-B" };
    const snapshot = {
      ...subtitles({ text: pair.source, isFinal: false, utteranceId: repeatedPair.utteranceId },
        { text: pair.translation, isFinal: false, utteranceId: repeatedPair.utteranceId }, [{ ...pair, createdAt: 1 }]),
      displayPair: repeatedPair,
      previewPair: repeatedPair,
    };
    expect(visibleLiveSubtitles(snapshot, bilingual, "en", true, false, true)).toEqual(
      pairRows.map(row => ({ ...row, utteranceId: repeatedPair.utteranceId })),
    );
    expect(snapshot.history).toHaveLength(1);
  });

  it.each([
    { targetLanguage: "original" as const, detectedLanguage: "en", source: "Current original." },
    { targetLanguage: "zh" as const, detectedLanguage: "zh", source: "当前原文。" },
  ])("shows current recognition when target=$targetLanguage already uses its language", ({ targetLanguage, detectedLanguage, source }) => {
    const snapshot = { ...subtitles({ text: source, isFinal: false, utteranceId: "synthetic-current" }), displayPair: pair };
    expect(visibleLiveSubtitles(snapshot, { ...bilingual, targetLanguage }, detectedLanguage, false, false, true))
      .toEqual([{ text: source, isFinal: false, kind: "source", utteranceId: "synthetic-current" }]);
  });

  it("uses one bilingual lane for equal pair text and preserves the translation-only lane", () => {
    const equalPair = { ...pair, translation: `  ${pair.source}\n` };
    const snapshot = { ...subtitles({ text: "", isFinal: false }), displayPair: equalPair };
    expect(visibleLiveSubtitles(snapshot, bilingual, "en", false, false)).toEqual([pairRows[0]]);
    expect(visibleLiveSubtitles(snapshot, { ...settings, subtitleDisplayMode: "translation" }, "en", false, false))
      .toEqual([{ ...pairRows[1], text: equalPair.translation }]);
  });

  it("only suppresses the live display pair when the latest history entry already contains it", () => {
    const snapshot = {
      ...subtitles({ text: "", isFinal: false }, { text: "", isFinal: false }, [
        { ...pair, createdAt: 1 },
        { source: "Another complete source.", translation: "另一条完整译文。", createdAt: 2 },
      ]),
      displayPair: pair,
    };
    expect(visibleLiveSubtitles(snapshot, bilingual, "en", false, false)).toEqual(pairRows);
  });

  it.each([
    [undefined, false],
    [null, false],
    [{ source: "  \n", translation: "\t " }, false],
    [{ source: "Visible original.", translation: "" }, true],
    [{ source: "", translation: "可见译文。" }, true],
  ] as const)("detects display-only content for %j", (displayPair, expected) => {
    expect(hasSubtitleContent({ ...subtitles({ text: "", isFinal: false }), displayPair })).toBe(expected);
  });

  it("detects a display-only pair in an independent source track", () => {
    const empty = subtitles({ text: "", isFinal: false });
    expect(hasSubtitleContent({ ...empty, tracks: [{
      ...empty, audioSource: "system", detectedLanguage: "en", isTranslationPending: false,
      isTranslationTimedOut: false, displayPair: pair,
    }] })).toBe(true);
  });
});

describe("realtime drafts after a confirmed display pair", () => {
  const previous = { source: "Synthetic previous source.", translation: "合成的上一条译文。" };
  const next = { source: "Synthetic next source grows across a long turn.", translation: "合成的新译文继续增长。" };
  const bilingual = { ...settings, subtitleDisplayMode: "bilingual" } as const;
  const snapshot = (retainHistory: boolean): SubtitleSnapshot => ({
    ...subtitles({ text: next.source, isFinal: false }, { text: next.translation, isFinal: false },
      retainHistory ? [{ ...previous, createdAt: 1 }] : []),
    previewPair: null,
    displayPair: previous,
    displayPairFinal: true,
  });
  const select = (value: SubtitleSnapshot, subtitleDisplayMode: "translation" | "bilingual" = "bilingual", showIntermediateSubtitles = true) =>
    visibleLiveSubtitles(value, { ...bilingual, subtitleDisplayMode, showIntermediateSubtitles }, "en", false, false,
      usesAtomicSubtitlePreview("googleGeminiLive"));

  it.each([false, true])("advances both realtime drafts after a final with retained history=%s", retainHistory => {
    const value = snapshot(retainHistory);
    const before = structuredClone(value);
    expect(select(value)).toEqual([
      { text: next.source, isFinal: false, kind: "source" },
      { text: next.translation, isFinal: false, kind: "translation" },
    ]);
    expect(select(value, "translation")).toEqual([
      { text: next.translation, isFinal: false, kind: "translation" },
    ]);
    value.translation.text += "更多合成文字。";
    expect(select(value, "translation")[0].text).toBe(value.translation.text);
    expect(value.history).toEqual(before.history);
    expect(value.displayPair).toEqual(before.displayPair);
    expect(value.displayPairFinal).toBe(true);
    expect(value.source).toEqual(before.source);
  });

  it.each([false, true])("does not attach the previous final translation to a new source with history=%s", retainHistory => {
    const value = snapshot(retainHistory);
    value.translation = { text: previous.translation, isFinal: true };
    expect(select(value)).toEqual([{ text: next.source, isFinal: false, kind: "source" }]);
    // Translation-only readers keep their last confirmed text until a new
    // translation exists; the source draft never replaces that lane.
    expect(select(value, "translation")).toEqual(retainHistory ? [] : [
      { text: previous.translation, isFinal: true, kind: "translation", isStable: true },
    ]);
  });

  it.each([false, true])("does not attach the previous final source to a translation-first draft with history=%s", retainHistory => {
    const value = snapshot(retainHistory);
    value.source = { text: previous.source, isFinal: true };
    expect(select(value)).toEqual([{ text: next.translation, isFinal: false, kind: "translation" }]);
  });

  it("preserves a different identified final source even when its wording repeats", () => {
    const value = snapshot(false);
    value.displayPair = { ...previous, utteranceId: "previous-turn" };
    value.source = { text: previous.source, isFinal: true, utteranceId: "current-turn" };
    value.translation.utteranceId = "current-turn";
    expect(select(value)).toEqual([
      { text: previous.source, isFinal: true, kind: "source", utteranceId: "current-turn" },
      { text: next.translation, isFinal: false, kind: "translation", utteranceId: "current-turn" },
    ]);
  });

  it.each([false, true])("keeps interim-off presentation final-only with retained history=%s", retainHistory => {
    const value = snapshot(retainHistory);
    expect(select(value, "bilingual", false)).toEqual(retainHistory ? [] : [
      { text: previous.source, isFinal: true, kind: "source", isStable: true },
      { text: previous.translation, isFinal: true, kind: "translation", isStable: true },
    ]);
  });

  it("preserves complete-pair holding for independent HTTP translation", () => {
    const value = snapshot(false);
    expect(visibleLiveSubtitles(value, bilingual, "en", false, false, true)).toEqual([
      { text: previous.source, isFinal: true, kind: "source", isStable: true },
      { text: previous.translation, isFinal: true, kind: "translation", isStable: true },
    ]);
  });

  it("shows a repeated realtime draft as a new unconfirmed reading", () => {
    const value = snapshot(true);
    value.source.text = previous.source;
    value.translation.text = previous.translation;
    expect(select(value, "translation")).toEqual([
      { text: previous.translation, isFinal: false, kind: "translation" },
    ]);
  });
});

describe("shared realtime projection", () => {
  const snapshot: SubtitleSnapshot = {
    source: { text: "Synthetic B.", isFinal: false, utteranceId: "B" },
    translation: { text: "Old A.", isFinal: true, utteranceId: "A" },
    history: [], previewPair: null,
    displayPair: { source: "Synthetic A.", translation: "Old A.", utteranceId: "A" }, displayPairFinal: true,
    realtimePreview: { source: { text: "Synthetic B.", isFinal: false, utteranceId: "B" }, translation: { text: "", isFinal: false } },
  };
  const settings = { sourceLanguage: "en", targetLanguage: "zh", subtitleDisplayMode: "bilingual" } as const;
  it("uses the Rust-selected lanes without attaching the prior translation", () => {
    expect(visibleLiveSubtitles(snapshot, settings, "en", false, false)).toEqual([
      { kind: "source", text: "Synthetic B.", isFinal: false, utteranceId: "B" },
    ]);
  });
  it("keeps a new owner's same-text source final while its translation is still pending", () => {
    const source = { text: "Synthetic A.", isFinal: true, utteranceId: "B" };
    const value = { ...snapshot,
      history: [{ source: "Synthetic A.", translation: "Old A.", createdAt: 1 }],
      realtimePreview: { source, translation: { text: "", isFinal: false } },
    };
    expect(visibleLiveSubtitles(value, settings, "en", false, false)).toEqual([
      { kind: "source", ...source },
    ]);
    expect(value.history).toHaveLength(1);
  });
  it("keeps a translation-first final even when its text repeats the preceding sentence", () => {
    const translation = { text: "Old A.", isFinal: true, utteranceId: "B" };
    const value = { ...snapshot,
      history: [{ source: "Synthetic A.", translation: "Old A.", createdAt: 1 }],
      realtimePreview: { source: { text: "", isFinal: false }, translation },
    };
    expect(visibleLiveSubtitles(value, settings, "en", false, false)).toEqual([
      { kind: "translation", ...translation },
    ]);
  });
  it("keeps complete pairs when interim display is off or the route is atomic", () => {
    for (const [showIntermediateSubtitles, atomic] of [[false, false], [true, true]]) {
      expect(visibleLiveSubtitles(snapshot, { ...settings, showIntermediateSubtitles }, "en", false, false, atomic)
        .map(row => row.text)).toEqual(["Synthetic A.", "Old A."]);
    }
  });
});
