import { describe, expect, it } from "vitest";
import { AUDIO3_RECOGNITION_LANGUAGE_CODES, type SettingsSnapshot } from "../../lib/types";
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
  recordSessionAudio: false,
  windowsAudioSource: "",
  showInDock: false,
  networkProxy: { mode: "system", url: null },
};

describe("overlay control panel model", () => {
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

  it.each(["deepL", "deepLX", "openAICompatible"] as const)("keeps %s route limits separate from Lite's expanded languages", route => {
    const settings = { ...BASE_SETTINGS,
      profiles: [{ ...BASE_SETTINGS.profiles[0], textTranslation: route }],
    };
    expect(overlayControlPanelModel(settings).sourceOptions).toEqual(["auto", "ja", "en", "ko", "zh"]);
    expect(overlayControlPanelModel(settings).sourceOptions).not.toContain("fr");
    expect(overlayControlPanelModel({ ...settings, targetLanguage: "original" }).sourceOptions)
      .toEqual(["auto", "ja", "en", "ko", "zh"]);
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
    expect(overlayControlPanelModel(changedRoute).sourceOptions).toEqual(["auto", "ja", "en", "ko", "zh"]);
  });

  it("omits translation modes when only original subtitles are requested", () => {
    const model = overlayControlPanelModel({
      ...BASE_SETTINGS,
      targetLanguage: "original",
    });
    expect(model.translationModeOptions).toEqual([]);
  });

  it("omits single-option OpenAI groups", () => {
    const model = overlayControlPanelModel({
      ...BASE_SETTINGS,
      activeProfileId: "openai",
    });
    expect(model.sourceOptions).toEqual([]);
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
