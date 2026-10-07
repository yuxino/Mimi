import { afterEach, beforeEach, expect, it } from "vitest";
import { I18N, providerDisplayName, setStoredUiLanguage } from "../../lib/i18n";
import type { ServiceProfile, SettingsSnapshot } from "../../lib/types";
import { translationService } from "./translationService";

const profile: ServiceProfile = {
  id: "synthetic-profile",
  name: "Synthetic configuration",
  provider: "alibabaCloud",
  credentialState: "present",
};

function settings(overrides: Partial<ServiceProfile> = {}) {
  return {
    profiles: [{ ...profile, ...overrides }],
    activeProfileId: profile.id,
    targetLanguage: "zh",
  } satisfies Pick<SettingsSnapshot, "profiles" | "activeProfileId" | "targetLanguage">;
}

beforeEach(() => setStoredUiLanguage("en"));
afterEach(() => setStoredUiLanguage("system"));

it.each([
  "alibabaCloud", "openAIRealtime", "googleGeminiLive", "azureOpenAIRealtime",
  "volcanoEngine", "tencentCloud", "baiduTranslate", "xAIRealtime",
] as const)("combines recognition and translation from the built-in %s service", provider => {
  const result = translationService(settings({ provider }));
  expect(result?.stages).toEqual([{ role: "combined", provider, label: providerDisplayName(provider) }]);
  expect(result?.detail).toContain(`Current configuration: ${profile.name}`);
  expect(result?.detail).toContain(`Speech recognition: ${providerDisplayName(provider)}`);
  expect(result?.detail).toContain(`Text translation: ${providerDisplayName(provider)}`);
});

it.each(["alibabaCloud", "customDashScopeASR", "customOpenAIASR", "appleSpeech"] as const)(
  "keeps %s recognition visible beside each independent translator", provider => {
    for (const [textTranslation, label] of [
      ["deepL", "DeepL"], ["deepLX", "DeepLX"], ["chatMock", "ChatMock"],
      ["openAICompatible", "OpenAI-compatible API"], ["apple", "Apple Translation"],
    ] as const) {
      const result = translationService(settings({ provider, textTranslation }));
      expect(result?.stages).toEqual([
        { role: "recognition", provider, label: providerDisplayName(provider) },
        { role: "translation", provider: textTranslation, label },
      ]);
      expect(result?.detail).toContain(`Speech recognition: ${providerDisplayName(provider)}`);
      expect(result?.detail).toContain(`Text translation: ${label}`);
    }
  },
);

it("resolves legacy DeepLX recognition to Alibaba and retains its independent translator", () => {
  const result = translationService(settings({ provider: "deepLX" }));
  expect(result?.stages).toEqual([
    { role: "recognition", provider: "alibabaCloud", label: "Alibaba Cloud" },
    { role: "translation", provider: "deepLX", label: "DeepLX" },
  ]);
  expect(result?.detail).toContain("Speech recognition: Alibaba Cloud");
  expect(translationService(settings({ provider: "deepLX", textTranslation: "followService" }))?.stages)
    .toEqual([{ role: "combined", provider: "alibabaCloud", label: "Alibaba Cloud" }]);
});

it("keeps recognition without claiming an unused translator when translation is disabled", () => {
  const result = translationService({ ...settings({ textTranslation: "chatMock" }), targetLanguage: "original" });
  expect(result?.stages).toEqual([{ role: "recognition", provider: "alibabaCloud", label: "Alibaba Cloud" }]);
  expect(result?.detail).toContain("Text translation: Original only");
  expect(result?.detail).not.toContain("ChatMock");
});

it.each(["customDashScopeASR", "customOpenAIASR", "appleSpeech"] as const)(
  "does not invent built-in translation for %s without an independent route", provider => {
    for (const textTranslation of [undefined, "followService"] as const) {
      const result = translationService(settings({ provider, textTranslation }));
      expect(result?.stages).toEqual([{ role: "recognition", provider, label: providerDisplayName(provider) }]);
      expect(result?.detail).toContain("Text translation: Original only");
    }
  },
);

it("keeps both actual routes when the display preference only hides translated subtitles", () => {
  const originalDisplay = { ...settings({ textTranslation: "deepL" }), subtitleDisplayMode: "original" as const };
  expect(translationService(originalDisplay)?.stages.map(stage => stage.provider)).toEqual(["alibabaCloud", "deepL"]);
});

it("omits unavailable profiles instead of borrowing a different configuration", () => {
  expect(translationService({ ...settings(), activeProfileId: "missing" })).toBeNull();
  expect(translationService({ ...settings(), profiles: [] })).toBeNull();
});

it.each(["zh", "en", "ja"] as const)("resolves both stages and original-only copy when the UI switches to %s", language => {
  setStoredUiLanguage(language);
  expect(translationService(settings({ textTranslation: "openAICompatible" }))?.stages).toEqual([
    { role: "recognition", provider: profile.provider, label: providerDisplayName(profile.provider) },
    { role: "translation", provider: "openAICompatible", label: I18N.settings.textTranslationOpenAICompatible },
  ]);
  expect(translationService({ ...settings(), targetLanguage: "original" })?.detail)
    .toContain(`${I18N.settings.textTranslationLabel}: ${I18N.overlay.originalOnly}`);
});

it.each(["deepL", "deepLX", "chatMock", "openAICompatible", "apple"] as const)(
  "uses the selected %s alias while retaining its protocol and recognition identities", textTranslation => {
    const result = translationService(settings({
      textTranslation,
      textTranslationNames: { [textTranslation]: "B 站 / 日本語 & 🌸" },
    }));
    expect(result?.stages).toEqual([
      { role: "recognition", provider: profile.provider, label: providerDisplayName(profile.provider) },
      { role: "translation", provider: textTranslation, label: "B 站 / 日本語 & 🌸" },
    ]);
    expect(result?.detail).toContain("Text translation: B 站 / 日本語 & 🌸");
  },
);

it("resolves aliases separately for the active profile and route on every update", () => {
  const first = settings({
    textTranslation: "openAICompatible",
    textTranslationNames: { openAICompatible: "Office translator", deepL: "Backup translator" },
  });
  const second: ServiceProfile = {
    ...profile, id: "second-profile", name: "Second configuration", provider: "customOpenAIASR", textTranslation: "openAICompatible",
    textTranslationNames: { openAICompatible: "Home translator" },
  };
  expect(translationService(first)?.stages[1].label).toBe("Office translator");
  const switched = translationService({ ...first, profiles: [...first.profiles, second], activeProfileId: second.id });
  expect(switched?.stages.map(stage => stage.label)).toEqual([providerDisplayName(second.provider), "Home translator"]);
  expect(switched?.detail).toContain("Current configuration: Second configuration");
  expect(translationService({ ...first, profiles: [{ ...first.profiles[0], textTranslation: "deepL" }] })?.stages[1].label)
    .toBe("Backup translator");
  expect(translationService({ ...first, profiles: [{ ...first.profiles[0], textTranslation: "chatMock" }] })?.stages[1].label)
    .toBe("ChatMock");
});

it.each(["", "  "])("falls back to the protocol name for a blank alias (%j)", name => {
  expect(translationService(settings({
    textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: name },
  }))?.stages[1].label).toBe(I18N.settings.textTranslationOpenAICompatible);
});

it("does not merge different services with matching display names", () => {
  const result = translationService(settings({
    textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: "Alibaba Cloud" },
  }));
  expect(result?.stages.map(stage => stage.label)).toEqual(["Alibaba Cloud", "Alibaba Cloud"]);
  expect(result?.stages.map(stage => stage.provider)).toEqual(["alibabaCloud", "openAICompatible"]);
});

it("does not leak inactive aliases into built-in or original-only translation", () => {
  const named = settings({
    textTranslation: "followService",
    textTranslationNames: { openAICompatible: "Unused translator" },
  });
  expect(translationService(named)?.stages).toHaveLength(1);
  expect(translationService(named)?.detail).not.toContain("Unused translator");
  const originalOnly = translationService({
    ...named, profiles: [{ ...named.profiles[0], textTranslation: "openAICompatible" }], targetLanguage: "original",
  });
  expect(originalOnly?.stages).toHaveLength(1);
  expect(originalOnly?.detail).toContain(I18N.overlay.originalOnly);
  expect(originalOnly?.detail).not.toContain("Unused translator");
});

it.each(["customDashScopeASR", "customOpenAIASR"] as const)(
  "uses an independent %s recognition name without borrowing the configuration or translator name", provider => {
    const result = translationService(settings({
      provider, name: "Whisper · Index", speechRecognitionName: "  Whisper / 日本語 🌸  ",
      textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: "Index · 本地" },
    }));
    expect(result?.stages).toEqual([
      { role: "recognition", provider, label: "Whisper / 日本語 🌸" },
      { role: "translation", provider: "openAICompatible", label: "Index · 本地" },
    ]);
    expect(result?.detail).toContain("Current configuration: Whisper · Index");
    expect(result?.detail).toContain("Speech recognition: Whisper / 日本語 🌸");
    expect(result?.detail).toContain("Text translation: Index · 本地");
  },
);

it.each(["customDashScopeASR", "customOpenAIASR"] as const)(
  "falls back to the localized %s protocol after clearing its recognition name", provider => {
    for (const language of ["zh", "en", "ja"] as const) {
      setStoredUiLanguage(language);
      for (const speechRecognitionName of [undefined, "", "   "]) {
        const result = translationService(settings({ provider, speechRecognitionName }));
        expect(result?.stages).toEqual([{ role: "recognition", provider, label: providerDisplayName(provider) }]);
        expect(result?.detail).toContain(`${I18N.settings.speechRecognition}: ${providerDisplayName(provider)}`);
      }
    }
  },
);

it.each(["customDashScopeASR", "customOpenAIASR"] as const)(
  "keeps the %s recognition alias in original-only and unconfigured translation modes", provider => {
    for (const textTranslation of [undefined, "followService", "chatMock"] as const) {
      const result = translationService({
        ...settings({ provider, speechRecognitionName: "Whisper", textTranslation, textTranslationNames: { chatMock: "Unused translator" } }),
        targetLanguage: textTranslation === "chatMock" ? "original" : "zh",
      });
      expect(result?.stages).toEqual([{ role: "recognition", provider, label: "Whisper" }]);
      expect(result?.detail).toContain("Text translation: Original only");
      expect(result?.detail).not.toContain("Unused translator");
    }
  },
);

it("refreshes the recognition alias on rename and profile selection without altering the translator", () => {
  const first = settings({
    provider: "customDashScopeASR", speechRecognitionName: "Office recognition", textTranslation: "chatMock",
    textTranslationNames: { chatMock: "Shared translator" },
  });
  const second: ServiceProfile = { ...first.profiles[0], id: "second-profile", speechRecognitionName: "Home recognition" };
  expect(translationService(first)?.stages.map(stage => stage.label)).toEqual(["Office recognition", "Shared translator"]);
  expect(translationService({ ...first, profiles: [{ ...first.profiles[0], speechRecognitionName: "Renamed recognition" }] })?.stages.map(stage => stage.label))
    .toEqual(["Renamed recognition", "Shared translator"]);
  expect(translationService({ ...first, profiles: [...first.profiles, second], activeProfileId: second.id })?.stages.map(stage => stage.label))
    .toEqual(["Home recognition", "Shared translator"]);
});

it.each(["alibabaCloud", "openAIRealtime", "deepLX"] as const)(
  "ignores an unsupported recognition alias for %s", provider => {
    const result = translationService(settings({ provider, speechRecognitionName: "Unused recognition name" }));
    expect(result?.stages[0].label).toBe(providerDisplayName(provider === "deepLX" ? "alibabaCloud" : provider));
    expect(result?.detail).not.toContain("Unused recognition name");
  },
);

it("keeps separate custom recognition and translation stages when their aliases match", () => {
  const result = translationService(settings({
    provider: "customOpenAIASR", speechRecognitionName: "My service", textTranslation: "openAICompatible",
    textTranslationNames: { openAICompatible: "My service" },
  }));
  expect(result?.stages).toEqual([
    { role: "recognition", provider: "customOpenAIASR", label: "My service" },
    { role: "translation", provider: "openAICompatible", label: "My service" },
  ]);
});
