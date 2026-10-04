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
] as const)("identifies the built-in %s service", provider => {
  const result = translationService(settings({ provider }));
  expect(result?.provider).toBe(provider);
  expect(result?.label).toBe(providerDisplayName(provider));
  expect(result?.detail).toContain(`Current configuration: ${profile.name}`);
  expect(result?.detail).toContain(`Text translation: ${providerDisplayName(provider)}`);
});

it.each(["alibabaCloud", "customDashScopeASR", "customOpenAIASR"] as const)(
  "identifies independent text translation instead of the %s recognition service", provider => {
    for (const [textTranslation, label] of [
      ["deepL", "DeepL"], ["deepLX", "DeepLX"], ["chatMock", "ChatMock"],
      ["openAICompatible", "OpenAI-compatible API"],
    ] as const) {
      const result = translationService(settings({ provider, textTranslation }));
      expect(result?.provider).toBe(textTranslation);
      expect(result?.label).toBe(label);
      expect(result?.detail).toContain(`Speech recognition: ${providerDisplayName(provider)}`);
      expect(result?.detail).toContain(`Text translation: ${label}`);
    }
  },
);

it("restores the translation route in legacy DeepLX snapshots", () => {
  expect(translationService(settings({ provider: "deepLX" }))?.provider).toBe("deepLX");
  expect(translationService(settings({ provider: "deepLX" }))?.label).toBe("DeepLX");
  expect(translationService(settings({ provider: "deepLX", textTranslation: "followService" }))?.provider).toBe("alibabaCloud");
});

it("does not claim the configured translator is used when translation is disabled", () => {
  const result = translationService({ ...settings({ textTranslation: "chatMock" }), targetLanguage: "original" });
  expect(result?.provider).toBeNull();
  expect(result?.label).toBe("Original only");
  expect(result?.detail).not.toContain("ChatMock");
});

it.each(["customDashScopeASR", "customOpenAIASR"] as const)(
  "does not invent built-in translation for %s without an independent route", provider => {
    expect(translationService(settings({ provider }))?.provider).toBeNull();
    expect(translationService(settings({ provider, textTranslation: "followService" }))?.label).toBe("Original only");
  },
);

it("keeps the translation route when only the subtitle display hides translations", () => {
  const originalDisplay = { ...settings({ textTranslation: "deepL" }), subtitleDisplayMode: "original" as const };
  expect(translationService(originalDisplay)?.provider).toBe("deepL");
});

it("omits unavailable profiles instead of borrowing a different configuration", () => {
  expect(translationService({ ...settings(), activeProfileId: "missing" })).toBeNull();
  expect(translationService({ ...settings(), profiles: [] })).toBeNull();
});

it("follows profile and route changes without retaining the previous service", () => {
  const first = settings({ textTranslation: "deepL" });
  const secondProfile: ServiceProfile = { ...profile, id: "second", name: "Second configuration", textTranslation: "chatMock" };
  const profiles = [...first.profiles, secondProfile];
  expect(translationService({ ...first, profiles })?.label).toBe("DeepL");
  expect(translationService({ ...first, profiles, activeProfileId: secondProfile.id })?.detail)
    .toContain("Current configuration: Second configuration");
  expect(translationService({ ...first, profiles, activeProfileId: secondProfile.id })?.label).toBe("ChatMock");
  expect(translationService(settings({ textTranslation: "openAICompatible" }))?.label).toBe("OpenAI-compatible API");
});

it.each(["zh", "en", "ja"] as const)("resolves service copy when the UI switches to %s", language => {
  setStoredUiLanguage(language);
  expect(translationService(settings({ textTranslation: "openAICompatible" }))?.label)
    .toBe(I18N.settings.textTranslationOpenAICompatible);
  expect(translationService({ ...settings(), targetLanguage: "original" })?.label)
    .toBe(I18N.overlay.originalOnly);
});

it.each(["deepL", "deepLX", "chatMock", "openAICompatible"] as const)(
  "uses the selected %s alias while retaining the protocol identity", textTranslation => {
    const result = translationService(settings({
      textTranslation,
      textTranslationNames: { [textTranslation]: "My translator" },
    }));
    expect(result?.provider).toBe(textTranslation);
    expect(result?.label).toBe("My translator");
    expect(result?.detail).toContain("Text translation: My translator");
    expect(result?.detail).toContain(`Speech recognition: ${providerDisplayName(profile.provider)}`);
  },
);

it("resolves aliases separately for the selected route and active profile", () => {
  const first = settings({
    textTranslation: "openAICompatible",
    textTranslationNames: { openAICompatible: "Office translator", deepL: "Backup translator" },
  });
  const second: ServiceProfile = {
    ...profile, id: "second-profile", textTranslation: "openAICompatible",
    textTranslationNames: { openAICompatible: "Home translator" },
  };
  expect(translationService(first)?.label).toBe("Office translator");
  expect(translationService({ ...first, profiles: [...first.profiles, second], activeProfileId: second.id })?.label)
    .toBe("Home translator");
  expect(translationService({ ...first, profiles: [{ ...first.profiles[0], textTranslation: "deepL" }] })?.label)
    .toBe("Backup translator");
  expect(translationService({ ...first, profiles: [{ ...first.profiles[0], textTranslation: "chatMock" }] })?.label)
    .toBe("ChatMock");
});

it.each(["", "  "])("falls back to the protocol name for a blank alias (%j)", name => {
  expect(translationService(settings({
    textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: name },
  }))?.label).toBe(I18N.settings.textTranslationOpenAICompatible);
});

it("does not leak inactive aliases into built-in or original-only translation labels", () => {
  const named = settings({
    textTranslation: "followService",
    textTranslationNames: { openAICompatible: "Unused translator" },
  });
  expect(translationService(named)?.label).toBe(providerDisplayName(profile.provider));
  expect(translationService(named)?.detail).not.toContain("Unused translator");
  const originalOnly = translationService({
    ...named, profiles: [{ ...named.profiles[0], textTranslation: "openAICompatible" }], targetLanguage: "original",
  });
  expect(originalOnly?.provider).toBeNull();
  expect(originalOnly?.label).toBe(I18N.overlay.originalOnly);
  expect(originalOnly?.detail).not.toContain("Unused translator");
});
