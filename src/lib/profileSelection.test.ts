import { afterEach, expect, it } from "vitest";
import { I18N, setStoredUiLanguage } from "./i18n";
import { profileLanguagePresetLabel } from "./profileLanguagePreset";
import { profileSelectionState } from "./profileSelection";
import type { ServiceProfile } from "./types";

const ready: ServiceProfile = { id: "ready", name: "Saved service", provider: "alibabaCloud", credentialState: "present" };

afterEach(() => setStoredUiLanguage("system"));

it.each(["zh", "zh-TW", "en", "ja", "de", "ko", "fr", "th"] as const)(
  "uses existing %s reasons without adding status text to ready choices", language => {
    setStoredUiLanguage(language);
    expect(profileSelectionState(ready, "zh")).toEqual({ disabled: false, description: undefined });
    expect(profileSelectionState({ ...ready, credentialState: "missing" }, "zh"))
      .toEqual({ disabled: true, description: I18N.settings.credentialMissing });
    expect(profileSelectionState({ ...ready, credentialState: "unavailable" }, "zh"))
      .toEqual({ disabled: true, description: I18N.settings.credentialUnavailable });
    expect(profileSelectionState({ ...ready, provider: "appleSpeech", credentialState: "missing" }, "zh"))
      .toEqual({ disabled: true, description: I18N.settings.appleSpeechNeedsSetup });
    expect(profileSelectionState({ ...ready, provider: "appleSpeech", credentialState: "unavailable", speechCredentialState: "unavailable" }, "zh"))
      .toEqual({ disabled: true, description: I18N.settings.appleSpeechUnavailable });
  },
);

it("distinguishes unavailable Apple text credentials from native speech for both target presets", () => {
  const profile: ServiceProfile = { ...ready, provider: "appleSpeech", credentialState: "unavailable",
    speechCredentialState: "present", textCredentialState: "unavailable", textTranslation: "deepL" };
  expect(profileSelectionState(profile, "zh")).toEqual({ disabled: true, description: I18N.settings.credentialUnavailable });
  expect(profileSelectionState(profile, "original")).toEqual({ disabled: false, description: undefined });
  const original: ServiceProfile = { ...profile, languagePreset: { sourceLanguage: "en", targetLanguage: "original" } };
  expect(profileSelectionState(original, "zh")).toEqual({ disabled: false, description: profileLanguagePresetLabel(original) });
  const translated: ServiceProfile = { ...profile, languagePreset: { sourceLanguage: "en", targetLanguage: "zh" } };
  expect(profileSelectionState(translated, "original")).toEqual({ disabled: true,
    description: `${I18N.settings.credentialUnavailable} · ${profileLanguagePresetLabel(translated)}` });
});

it("does not infer unavailable native speech from an aggregate state with no speech slot", () => {
  const profile: ServiceProfile = { ...ready, provider: "appleSpeech", credentialState: "unavailable" };
  for (const target of ["zh", "original"] as const) {
    expect(profileSelectionState(profile, target)).toEqual({ disabled: true, description: I18N.settings.credentialUnavailable });
  }
});

it("requires a workspace only for Alibaba simultaneous translation and recovers after setup", () => {
  const profile: ServiceProfile = { ...ready, alibabaRealtimeMode: "liveTranslate", alibabaWorkspaceId: "  " };
  expect(profileSelectionState(profile, "zh")).toEqual({ disabled: true, description: I18N.settings.appleSpeechNeedsSetup });
  expect(profileSelectionState({ ...profile, alibabaWorkspaceId: " workspace-id " }, "zh").disabled).toBe(false);
  expect(profileSelectionState({ ...profile, alibabaRealtimeMode: "disabled" }, "zh").disabled).toBe(false);
});

it.each(["appleSpeech", "customDashScopeASR", "customOpenAIASR"] as const)(
  "uses the candidate %s preset before the current target and preserves its label", provider => {
    const profile: ServiceProfile = { ...ready, provider, credentialState: "missing", speechCredentialState: "present", textCredentialState: "missing" };
    expect(profileSelectionState(profile, "original").disabled).toBe(false);
    expect(profileSelectionState(profile, "zh").disabled).toBe(true);
    const original: ServiceProfile = { ...profile, languagePreset: { sourceLanguage: "en", targetLanguage: "original" } };
    expect(profileSelectionState(original, "zh")).toEqual({ disabled: false, description: profileLanguagePresetLabel(original) });
    const translated: ServiceProfile = { ...profile, languagePreset: { sourceLanguage: "en", targetLanguage: "zh" } };
    const selection = profileSelectionState(translated, "original");
    expect(selection.disabled).toBe(true);
    expect(selection.description).toContain(profileLanguagePresetLabel(translated));
  },
);

it.each(["customDashScopeASR", "customOpenAIASR", "deepLX"] as const)(
  "accepts a ready %s snapshot without requiring secret fields in the frontend", provider => {
    expect(profileSelectionState({ ...ready, provider, textTranslation: "chatMock" }, "zh"))
      .toEqual({ disabled: false, description: undefined });
  },
);
