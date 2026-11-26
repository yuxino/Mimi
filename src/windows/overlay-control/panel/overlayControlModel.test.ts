import { describe, expect, it } from "vitest";
import type { SettingsSnapshot, SourceLanguage } from "../../../lib/types";
import {
  sourceLanguagesForSettings,
  targetLanguagesForSettings,
} from "../../../lib/providers/providerCapabilities";
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

  it("derives source and target pickers from the active profile catalog", () => {
    const alibaba = overlayControlPanelModel(BASE_SETTINGS);
    expect(alibaba.sourceOptions).toEqual(sourceLanguagesForSettings(BASE_SETTINGS));
    expect(alibaba.targetOptions).toEqual(targetLanguagesForSettings(BASE_SETTINGS));

    const openaiSettings = { ...BASE_SETTINGS, activeProfileId: "openai" };
    const openai = overlayControlPanelModel(openaiSettings);
    expect(openai.sourceOptions).toEqual(["auto"]);
    expect(openai.targetOptions).toEqual(targetLanguagesForSettings(openaiSettings));
  });

  it("applies stamped native language capabilities and falls back when expired", () => {
    const native = {
      profileId: "ali", provider: "alibabaCloud", textTranslation: "followService", targetLanguage: "zh",
      sourceLanguages: ["auto", "fr"], targetLanguages: ["original", "zh", "fr"],
    } as const;
    const stamped = overlayControlPanelModel({ ...BASE_SETTINGS, languageCapabilities: native });
    expect(stamped.sourceOptions).toEqual(["auto", "fr"]);
    expect(stamped.targetOptions).toEqual(["original", "zh", "fr"]);

    const expired = overlayControlPanelModel({
      ...BASE_SETTINGS,
      languageCapabilities: { ...native, profileId: "old-profile" },
    });
    expect(expired.sourceOptions).toEqual(sourceLanguagesForSettings(BASE_SETTINGS));
    expect(expired.targetOptions).toEqual(targetLanguagesForSettings(BASE_SETTINGS));
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
