import { describe, expect, it } from "vitest";
import { AUDIO3_RECOGNITION_LANGUAGE_CODES, type SettingsSnapshot, type SourceLanguage } from "../../lib/types";
import { sourceLanguagesForSettings } from "../../lib/providerCapabilities";
import { overlayControlPanelModel } from "./overlayControlModel";

const BASE_SETTINGS: SettingsSnapshot = {
  profiles: [
    {
      id: "ali",
      name: "Alibaba Cloud",
      provider: "alibabaCloud",
      credentialState: "present",
    },
    {
      id: "openai",
      name: "OpenAI Realtime",
      provider: "openAIRealtime",
      credentialState: "present",
    },
  ],
  activeProfileId: "ali",
  sourceLanguage: "auto",
  targetLanguage: "zh",
  translationMode: "highQuality",
  fontSize: 18,
  subtitleBackgroundOpacity: 80,
  subtitleColor: "white",
  subtitleAlignment: "center",
  subtitleDisplayMode: "translation",
  showSubtitleDividers: false,
  pulseAnimation: null,
  pulseStyle: "ribbon",
  subtitleAnimation: null,
  subtitleBlendsWithBackground: false,
  isOverlayLocked: false,
  uiLanguage: null,
  retainSessionHistory: false,
  recordSessionAudio: false, audioInput: "system",
  windowsAudioSource: "",
  systemAudioTarget: { kind: "system" },
  showInDock: false,
  networkProxy: { mode: "system", url: null },
};

describe("overlay control panel model", () => {
  it.each([
    { ready: [], current: "fr", expected: [] },
    { ready: ["en"], current: "en", expected: ["en"] },
    { ready: ["en"], current: "fr", expected: ["en"] },
    { ready: ["en", "ja"], current: "fr", expected: ["en", "ja"] },
  ] satisfies { ready: SourceLanguage[]; current: SourceLanguage; expected: SourceLanguage[] }[])("keeps only ready Apple options and permits recovery from $current to $ready", ({ ready, current, expected }) => {
    const settings: SettingsSnapshot = { ...BASE_SETTINGS, sourceLanguage: current, targetLanguage: "original", activeProfileId: "apple",
      profiles: [{ id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "present" }],
      languageCapabilities: { profileId: "apple", provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original",
        sourceLanguages: ready, targetLanguages: ["original"] } };
    expect(overlayControlPanelModel(settings).sourceOptions).toEqual(expected);
    expect(settings.sourceLanguage).toBe(current);
  });

  it("shows Alibaba recognition choices without a redundant mode picker", () => {
    const model = overlayControlPanelModel(BASE_SETTINGS);
    expect(model.sourceOptions).toHaveLength(25);
    expect(model.sourceOptions).toEqual(sourceLanguagesForSettings(BASE_SETTINGS));
    expect(model.sourceOptions).toContain("fr");
    expect(model.sourceOptions).not.toContain("no");
    expect(model.translationModeOptions).toEqual([]);
    expect(model.effectiveTranslationMode).toBe("turbo");
  });

  it("keeps Turbo for a manually selected source without a mode picker", () => {
    const model = overlayControlPanelModel({
      ...BASE_SETTINGS,
      sourceLanguage: "ja",
    });
    expect(model.translationModeOptions).toEqual([]);
  });

  it("offers the complete translated list without depending on the selected extended source", () => {
    const settings = { ...BASE_SETTINGS, sourceLanguage: "fr" as const };
    expect(overlayControlPanelModel(settings).sourceOptions)
      .toEqual(sourceLanguagesForSettings(BASE_SETTINGS));
    expect(overlayControlPanelModel(settings).sourceOptions).toHaveLength(25);
  });

  it("offers all 30 Audio3 languages plus automatic only in Original mode", () => {
    const settings = { ...BASE_SETTINGS, sourceLanguage: "no" as const, targetLanguage: "original" as const };
    expect(overlayControlPanelModel(settings).sourceOptions)
      .toEqual(["auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES]);
    expect(overlayControlPanelModel(settings).sourceOptions).toHaveLength(31);
    expect(overlayControlPanelModel(settings).sourceOptions).toContain("no");
  });

  it.each(["deepL", "deepLX"] as const)("keeps %s route limits separate from Lite's expanded languages", route => {
    const settings = { ...BASE_SETTINGS,
      profiles: [{ ...BASE_SETTINGS.profiles[0], textTranslation: route }],
    };
    expect(overlayControlPanelModel(settings).sourceOptions).toEqual(sourceLanguagesForSettings(settings));
    expect(overlayControlPanelModel(settings).sourceOptions).toContain("fr");
    expect(overlayControlPanelModel({ ...settings, targetLanguage: "original" }).sourceOptions)
      .toEqual(sourceLanguagesForSettings({ ...settings, targetLanguage: "original" }));
  });

  it.each(["openAICompatible", "chatMock"] as const)("keeps all 31 recognition choices with the %s text route", route => {
    const settings = { ...BASE_SETTINGS,
      profiles: [{ ...BASE_SETTINGS.profiles[0], textTranslation: route }],
    };
    for (const targetLanguage of ["zh", "original"] as const) {
      const model = overlayControlPanelModel({ ...settings, targetLanguage });
      expect(model.sourceOptions).toEqual(["auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES]);
      expect(model.sourceOptions).toHaveLength(31);
      expect(model.sourceOptions).toContain("no");
    }
  });

  const native = {
    profileId: "ali", provider: "alibabaCloud", textTranslation: "followService", targetLanguage: "zh",
    sourceLanguages: ["auto", "fr"], targetLanguages: ["original", "zh", "fr"],
  } as const;

  it("uses valid native language options instead of the older local range", () => {
    const settings = { ...BASE_SETTINGS, languageCapabilities: native };
    expect(overlayControlPanelModel(settings).sourceOptions).toEqual(["auto", "fr"]);
  });

  it.each([
    { profileId: "old-profile" },
    { provider: "deepLX" as const },
    { textTranslation: "deepL" as const },
    { targetLanguage: "original" as const },
  ])("falls back to the full current route for an expired native stamp %j", changed => {
    const settings = { ...BASE_SETTINGS, languageCapabilities: { ...native, ...changed } };
    expect(overlayControlPanelModel(settings).sourceOptions).toEqual(sourceLanguagesForSettings(BASE_SETTINGS));
    expect(overlayControlPanelModel(settings).sourceOptions).toHaveLength(25);
  });

  it("uses the appropriate full fallback for missing capabilities, missing profiles and changed targets", () => {
    for (const settings of [
      { ...BASE_SETTINGS, languageCapabilities: undefined },
      { ...BASE_SETTINGS, activeProfileId: "removed", languageCapabilities: native },
    ]) {
      expect(overlayControlPanelModel(settings).sourceOptions).toHaveLength(25);
    }
    const original = { ...BASE_SETTINGS, targetLanguage: "original" as const, languageCapabilities: native };
    expect(overlayControlPanelModel(original).sourceOptions).toHaveLength(31);
    expect(overlayControlPanelModel(original).sourceOptions).toContain("no");
  });

  it("rejects empty native lists atomically and does not reuse a stamp after a route edit", () => {
    const empty = { ...BASE_SETTINGS, languageCapabilities: { ...native, sourceLanguages: [] } };
    expect(overlayControlPanelModel(empty).sourceOptions).toHaveLength(25);
    const changedRoute = { ...BASE_SETTINGS, languageCapabilities: native,
      profiles: [{ ...BASE_SETTINGS.profiles[0], textTranslation: "deepLX" as const }],
    };
    expect(overlayControlPanelModel(changedRoute).sourceOptions).toEqual(sourceLanguagesForSettings(changedRoute));
  });

  it("omits translation modes when only original subtitles are requested", () => {
    const model = overlayControlPanelModel({
      ...BASE_SETTINGS,
      targetLanguage: "original",
    });
    expect(model.translationModeOptions).toEqual([]);
  });

  it("retains the automatic source alongside OpenAI target choices", () => {
    const model = overlayControlPanelModel({
      ...BASE_SETTINGS,
      activeProfileId: "openai",
    });
    expect(model.sourceOptions).toEqual(["auto"]);
    expect(model.translationModeOptions).toEqual([]);
    expect(model.effectiveTranslationMode).toBe("turbo");
  });

  it("derives immersive and position-lock switch state", () => {
    expect(overlayControlPanelModel(BASE_SETTINGS).immersiveModeEnabled).toBe(
      false,
    );
    expect(overlayControlPanelModel(BASE_SETTINGS).overlayLocked).toBe(false);

    const enabled = overlayControlPanelModel({
      ...BASE_SETTINGS,
      subtitleBlendsWithBackground: true,
      isOverlayLocked: true,
    });
    expect(enabled.immersiveModeEnabled).toBe(true);
    expect(enabled.overlayLocked).toBe(true);
  });
});
