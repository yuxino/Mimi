import { expect, it } from "vitest";
import type { SubtitleSnapshot } from "../../lib/types";
import { emptyStateText, languageStatus, isWaitingForFinalTranslation, visibleLiveSubtitles } from "./overlayModel";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { SOURCE_LANGUAGE_DISPLAY_NAMES } from "../../lib/types";
import { selectSessionErrorMessage, useStore } from "../../lib/store";

const subtitles: SubtitleSnapshot = {
  source: { text: "Synthetic current source.", isFinal: false, utteranceId: "current" },
  translation: { text: "Synthetic older translation.", isFinal: false, utteranceId: "old" },
  previewPair: { source: "Synthetic paired source.", translation: "Synthetic paired translation." },
  history: [],
};

it.each(["zh", "en", "ja"] as const)("shares actionable Apple error messages across overlay and session controls in %s", language => {
  setStoredUiLanguage(language);
  try {
    const initial = useStore.getState();
    for (const [message, expected] of [["apple_speech_assets_missing", I18N.settings.appleSpeechAssetsMissing], ["apple_speech_language_unsupported", I18N.settings.appleSpeechLanguageUnsupported], ["apple_speech_audio_failed", I18N.settings.appleSpeechRecognitionFailed]] as const) {
      const session = { ...initial.session, isPaused: false, status: { kind: "error" as const, message } };
      expect(emptyStateText(session, initial.settings)).toBe(expected);
      expect(selectSessionErrorMessage({ ...initial, session })).toBe(expected);
    }
  } finally { setStoredUiLanguage("system"); }
});

it.each(["original", "translation", "bilingual"] as const)("keeps a single recognition lane for an exact expanded same-language pair in %s", (mode) => {
  const settings = { sourceLanguage: "fr", targetLanguage: "fr", subtitleDisplayMode: mode } as const;
  expect(isWaitingForFinalTranslation(settings, null, true)).toBe(false);
  expect(visibleLiveSubtitles(subtitles, settings, null, true, false, true))
    .toEqual([{ text: subtitles.source.text, isFinal: false, kind: mode === "translation" ? "translation" : "source", utteranceId: "current" }]);
});

it("keeps the empty-state translation feedback tied to the pending source, not the first detected language", () => {
  const initial = useStore.getState();
  const tracks = (["system", "microphone"] as const).map(audioSource => ({
    ...initial.session.subtitles, audioSource, detectedLanguage: audioSource === "system" ? "zh" : "en",
    isTranslationPending: audioSource === "microphone", isTranslationTimedOut: false,
  }));
  const session = { ...initial.session, status: { kind: "listening" as const }, isPaused: false,
    detectedLanguage: "zh", isTranslationPending: true, subtitles: { ...initial.session.subtitles, tracks },
  };
  const settings = { ...initial.settings, sourceLanguage: "auto" as const, targetLanguage: "zh" as const, audioInput: "both" as const };
  expect(emptyStateText(session, settings)).toBe(I18N.overlay.translatingEmpty);
  expect(emptyStateText(session, { ...settings, targetLanguage: "original" })).toBe(I18N.overlay.listeningEmpty);
  expect(emptyStateText({ ...session, isPaused: true }, settings)).toBe(I18N.overlay.paused);
});

it.each(["zh", "en", "ja"] as const)("uses a neutral automatic summary for independent source languages in %s", language => {
  setStoredUiLanguage(language);
  try {
    const settings = { ...useStore.getState().settings, audioInput: "both" as const, sourceLanguage: "auto" as const };
    for (const detected of [null, "zh", "en"]) {
      expect(languageStatus(settings, detected)?.source).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.auto);
    }
  } finally { setStoredUiLanguage("system"); }
});

it("keeps Simplified-to-Traditional Chinese as MT instead of treating Chinese scripts as identical", () => {
  const settings = { sourceLanguage: "zh", targetLanguage: "zh_tw", subtitleDisplayMode: "bilingual" } as const;
  expect(isWaitingForFinalTranslation(settings, "zh", true)).toBe(true);
  expect(visibleLiveSubtitles(subtitles, settings, "zh", true, false, true))
    .toEqual([
      { kind: "source", text: subtitles.previewPair!.source, isFinal: false, isStable: true },
      { kind: "translation", text: subtitles.previewPair!.translation, isFinal: false, isStable: true },
    ]);
  expect(isWaitingForFinalTranslation({ ...settings, sourceLanguage: "auto" }, null, true)).toBe(true);
});


it("shows the translated lane and keeps pending feedback for bilingual direction mode", () => {
  const settings = { sourceLanguage: "zh_en", targetLanguage: "zh_en", subtitleDisplayMode: "translation" } as const;
  const current: SubtitleSnapshot = { source: { text: "Synthetic source.", isFinal: false }, translation: { text: "Synthetic translated text.", isFinal: false }, history: [] };
  expect(isWaitingForFinalTranslation(settings, null, true)).toBe(true);
  expect(isWaitingForFinalTranslation(settings, "zh_en", true)).toBe(true);
  expect(visibleLiveSubtitles(current, settings, null, false, false)).toEqual([
    { text: current.translation.text, kind: "translation", isFinal: false },
  ]);
});

it.each(["zh", "en", "ja"] as const)("uses the same bilingual direction labels for the %s overlay as settings", language => {
  setStoredUiLanguage(language);
  try {
    const settings = { ...useStore.getState().settings, activeProfileId: "volcano", sourceLanguage: "zh_en", targetLanguage: "zh_en",
      profiles: [{ id: "volcano", name: "Volcano", provider: "volcanoEngine", credentialState: "present" }] } as const;
    expect(languageStatus({ ...settings, profiles: [...settings.profiles] }, null)).toEqual({ source: I18N.settings.recognitionVolcanoBilingual, separator: I18N.overlay.separator, target: I18N.settings.recognitionVolcanoBilingual });
  } finally { setStoredUiLanguage("system"); }
});
