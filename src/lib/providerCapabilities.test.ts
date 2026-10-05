import contract from "../../shared/translation-contracts.json";
import audio3 from "../../src-tauri/src/core/protocols/audio3.rs?raw";
import qwenMt from "../../src-tauri/src/core/protocols/qwen_mt.rs?raw";
import { describe, expect, it } from "vitest";
import { AUDIO3_RECOGNITION_LANGUAGE_CODES, QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES, type SettingsSnapshot } from "./types";
import {
  SERVICE_PROVIDERS,
  capabilitiesForProvider,
  OPENAI_TRANSLATION_TARGETS,
  effectiveProviderForProfile,
  textTranslationForProfile,
  activeServiceProfile,
  capabilitiesForProfile,
  credentialStateForTarget,
  effectiveTranslationModeForSettings,
  sourceLanguagesForSettings,
  subtitlePreferencesChanged,
  targetLanguageAfterSourceSwitch,
  targetLanguagesForSettings,
  translationModesForSettings,
} from "./providerCapabilities";

it("requires only custom speech credentials for Original while translated targets require both stages", () => {
  const profile = { id: "custom", name: "Custom", provider: "customOpenAIASR", credentialState: "missing", speechCredentialState: "present", textCredentialState: "missing", textTranslation: "deepL" } as const;
  expect(credentialStateForTarget(profile, "original")).toBe("present");
  expect(credentialStateForTarget(profile, "zh")).toBe("missing");
  expect(capabilitiesForProfile(profile).targetLanguages).toEqual(["original", "zh", "en", "ja"]);
  expect(capabilitiesForProfile({ ...profile, textTranslation: "followService" }).targetLanguages).toEqual(["original"]);
});

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
      credentialState: "missing",
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

it("uses only this Mac's stamped Apple languages without inventing automatic detection or a fallback", () => {
  const apple = { id: "apple", provider: "appleSpeech", name: "Apple Speech", credentialState: "missing", speechCredentialState: "present", textTranslation: "openAICompatible" } as const;
  const base = { ...BASE_SETTINGS, profiles: [apple], activeProfileId: apple.id };
  expect(sourceLanguagesForSettings(base)).toEqual([]);
  const languageCapabilities = { profileId: apple.id, provider: apple.provider, textTranslation: apple.textTranslation, targetLanguage: "zh", sourceLanguages: ["en", "fr"], targetLanguages: ["original", "zh", "en", "ja"] } as const;
  expect(sourceLanguagesForSettings({ ...base, languageCapabilities })).toEqual(["en", "fr"]);
  expect(sourceLanguagesForSettings({ ...base, languageCapabilities: { ...languageCapabilities, sourceLanguages: ["auto"] } })).toEqual([]);
  expect(sourceLanguagesForSettings({ ...base, languageCapabilities: { ...languageCapabilities, sourceLanguages: [] } })).toEqual([]);
  expect(sourceLanguagesForSettings({ ...base, languageCapabilities: { ...languageCapabilities, profileId: "old" } })).toEqual([]);
  expect(credentialStateForTarget(apple, "original")).toBe("present");
  expect(credentialStateForTarget(apple, "zh")).toBe("missing");
});

it.each(["deepL", "deepLX"] as const)("intersects Apple sources with the %s text encoder while keeping original-only choices", route => {
  const profile = { id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "present", textTranslation: route } as const;
  const languageCapabilities = { profileId: profile.id, provider: profile.provider, textTranslation: route, targetLanguage: "en", sourceLanguages: ["en", "fr"], targetLanguages: ["original", "zh", "en", "ja"] } as const;
  const settings = { ...BASE_SETTINGS, profiles: [profile], activeProfileId: profile.id, targetLanguage: "en" as const, languageCapabilities };
  expect(sourceLanguagesForSettings(settings)).toEqual(["en"]);
  expect(sourceLanguagesForSettings({ ...settings, targetLanguage: "original", languageCapabilities: { ...languageCapabilities, targetLanguage: "original" } })).toEqual(["en", "fr"]);
});

describe("provider capabilities", () => {
  it("keeps language controls and only Turbo for manual Alibaba input", () => {
    const settings = { ...BASE_SETTINGS, sourceLanguage: "ja" as const };
    expect(sourceLanguagesForSettings(settings)).toHaveLength(25);
    expect(sourceLanguagesForSettings(settings)).toContain("fr");
    expect(sourceLanguagesForSettings(settings)).not.toContain("no");
    expect(targetLanguagesForSettings(settings)).toEqual(["original", ...QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES]);
    expect(translationModesForSettings(settings)).toEqual(["turbo"]);
  });

  it("normalizes legacy Alibaba preferences to Turbo for automatic detection", () => {
    expect(translationModesForSettings(BASE_SETTINGS)).toEqual(["turbo"]);
    expect(effectiveTranslationModeForSettings(BASE_SETTINGS)).toBe("turbo");
  });

  it("limits OpenAI Realtime to auto, supported targets, and turbo", () => {
    const settings = { ...BASE_SETTINGS, activeProfileId: "openai" };

    expect(activeServiceProfile(settings)?.provider).toBe("openAIRealtime");
    expect(sourceLanguagesForSettings(settings)).toEqual(["auto"]);
    expect(targetLanguagesForSettings(settings)).toEqual(OPENAI_TRANSLATION_TARGETS);
    expect(translationModesForSettings(settings)).toEqual(["turbo"]);
    expect(effectiveTranslationModeForSettings(settings)).toBe("turbo");
  });

  it("keeps the full target set for Alibaba and OpenAI automatic input", () => {
    expect(targetLanguagesForSettings(BASE_SETTINGS)).toEqual(["original", ...QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES]);
    expect(
      targetLanguagesForSettings({
        ...BASE_SETTINGS,
        activeProfileId: "openai",
        sourceLanguage: "auto",
      }),
    ).toEqual(OPENAI_TRANSLATION_TARGETS);
  });

  it.each([
    ["zh", ["en", "ja"]],
    ["en", ["zh", "ja"]],
    ["ja", ["zh", "en"]],
  ] as const)(
    "removes the %s target for providers with an explicit source",
    (sourceLanguage, expectedTargets) => {
      expect(
        targetLanguagesForSettings({
          ...BASE_SETTINGS,
          profiles: [
            {
              id: "tencent",
              name: "Tencent Cloud",
              provider: "tencentCloud",
              credentialState: "missing",
            },
          ],
          activeProfileId: "tencent",
          sourceLanguage,
        }),
      ).toEqual(expectedTargets);
    },
  );

  it("keeps OpenAI's effective mode on turbo for a stale stored mode", () => {
    const settings = {
      ...BASE_SETTINGS,
      activeProfileId: "openai",
      translationMode: "lowLatency" as const,
    };

    expect(effectiveTranslationModeForSettings(settings)).toBe("turbo");
  });

  it("lists every provider exactly once with custom recognition protocols after built-in services", () => {
    expect(new Set(SERVICE_PROVIDERS).size).toBe(11);
    expect(SERVICE_PROVIDERS).toEqual([
      "alibabaCloud",
      "googleGeminiLive",
      "volcanoEngine",
      "tencentCloud",
      "baiduTranslate",
      "openAIRealtime",
      "azureOpenAIRealtime",
      "xAIRealtime",
      "appleSpeech",
      "customDashScopeASR",
      "customOpenAIASR",
    ]);
  });

  it("uses automatic recognition only where the official protocol supports it", () => {
    const automatic = {
      ...BASE_SETTINGS,
      profiles: [
        {
          id: "google",
          name: "Google Gemini",
          provider: "googleGeminiLive" as const,
          credentialState: "missing" as const,
        },
      ],
      activeProfileId: "google",
    };
    expect(sourceLanguagesForSettings(automatic)).toEqual(["auto"]);
    expect(effectiveTranslationModeForSettings(automatic)).toBe("turbo");

    const explicit = {
      ...automatic,
      profiles: [
        {
          ...automatic.profiles[0],
          provider: "volcanoEngine" as const,
        },
      ],
    };
    expect(sourceLanguagesForSettings(explicit)).toEqual(["ja", "en", "zh"]);

    for (const provider of ["tencentCloud", "baiduTranslate"] as const) {
      expect(
        sourceLanguagesForSettings({
          ...automatic,
          profiles: [{ ...automatic.profiles[0], provider }],
        }),
      ).toEqual(["ja", "en", "ko", "zh"]);
    }
  });

  it("keeps translation enabled when an explicit provider switches to Chinese", () => {
    const settings = {
      ...BASE_SETTINGS,
      profiles: [
        {
          id: "tencent",
          name: "Tencent Cloud",
          provider: "tencentCloud" as const,
          credentialState: "missing" as const,
        },
      ],
      activeProfileId: "tencent",
      sourceLanguage: "ja" as const,
      targetLanguage: "en" as const,
    };

    expect(targetLanguageAfterSourceSwitch(settings, "zh")).toBe("en");
    expect(
      targetLanguageAfterSourceSwitch(
        { ...settings, targetLanguage: "zh" },
        "zh",
      ),
    ).toBe("en");
  });

  it("falls back to Alibaba capabilities when the active id is stale", () => {
    const settings = { ...BASE_SETTINGS, activeProfileId: "missing" };
    expect(activeServiceProfile(settings)).toBeUndefined();
    expect(sourceLanguagesForSettings(settings)).toContain("ko");
  });

  it("reports when profile selection normalizes subtitle preferences", () => {
    expect(
      subtitlePreferencesChanged(BASE_SETTINGS, {
        ...BASE_SETTINGS,
        sourceLanguage: "auto",
        targetLanguage: "zh",
        translationMode: "turbo",
      }),
    ).toBe(true);
    expect(subtitlePreferencesChanged(BASE_SETTINGS, { ...BASE_SETTINGS })).toBe(
      false,
    );
  });
});


it("keeps DeepLX out of speech-service choices while preserving legacy configurations", () => {
  expect(SERVICE_PROVIDERS).not.toContain("deepLX");
  const legacy = { id: "old", name: "Existing", provider: "deepLX", credentialState: "present" } as const;
  expect(textTranslationForProfile(legacy)).toBe("deepLX");
  expect(effectiveProviderForProfile(legacy)).toBe("deepLX");
  expect(effectiveProviderForProfile({ ...legacy, textTranslation: "followService" })).toBe("alibabaCloud");
});

it("applies actual Audio3+DeepLX capabilities only when selected", () => {
  const settings: SettingsSnapshot = { ...BASE_SETTINGS, profiles: [{ ...BASE_SETTINGS.profiles[0], textTranslation: "deepLX" }] };
  expect(targetLanguagesForSettings(settings)).toEqual(["zh", "en", "ja"]);
  expect(translationModesForSettings(settings)).toEqual(["turbo"]);
  expect(effectiveTranslationModeForSettings(settings)).toBe("turbo");
  expect(effectiveProviderForProfile({ ...BASE_SETTINGS.profiles[1], textTranslation: "deepLX" })).toBe("openAIRealtime");
});

it("keeps Alibaba recognition and Original mode when using official DeepL", () => {
  const profile = { ...BASE_SETTINGS.profiles[0], textTranslation: "deepL" as const };
  const settings: SettingsSnapshot = { ...BASE_SETTINGS, profiles: [profile] };
  expect(textTranslationForProfile(profile)).toBe("deepL");
  expect(effectiveProviderForProfile(profile)).toBe("alibabaCloud");
  expect(sourceLanguagesForSettings(settings)).toEqual(["auto", "ja", "en", "ko", "zh"]);
  expect(targetLanguagesForSettings(settings)).toEqual(["original", "zh", "en", "ja"]);
  expect(targetLanguagesForSettings(settings)).toContain("original");
});

it("offers all recognition hints only for Original and validates the translated intersection separately", () => {
  const original = { ...BASE_SETTINGS, targetLanguage: "original" as const };
  expect(sourceLanguagesForSettings(original)).toEqual(["auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES]);
  for (const code of ["no", "el", "ro", "bg", "hr", "sk"] as const) {
    expect(sourceLanguagesForSettings(original)).toContain(code);
    expect(sourceLanguagesForSettings(BASE_SETTINGS)).not.toContain(code);
  }
  for (const code of ["zh_tw", "he", "ur", "bn", "tr", "km", "fa"] as const) {
    expect(targetLanguagesForSettings(BASE_SETTINGS)).toContain(code);
    expect(sourceLanguagesForSettings(original)).not.toContain(code);
  }
});

it("allows explicit Chinese to English and keeps the full Alibaba target set", () => {
  const settings = { ...BASE_SETTINGS, sourceLanguage: "zh" as const, targetLanguage: "en" as const };
  expect(targetLanguagesForSettings(settings)).toEqual(["original", ...QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES]);
  expect(targetLanguagesForSettings(settings)).toContain("en");
  expect(targetLanguagesForSettings(settings)).toContain("zh_tw");
});

it("keeps inactive profile routes isolated, including migrated legacy DeepLX profiles", () => {
  for (const provider of ["alibabaCloud", "deepLX"] as const) {
    const profile = { ...BASE_SETTINGS.profiles[0], provider };
    expect(capabilitiesForProfile({ ...profile, textTranslation: "deepL" }).sourceLanguages).toEqual(["auto", "ja", "en", "ko", "zh"]);
    expect(capabilitiesForProfile({ ...profile, textTranslation: "deepL" }).targetLanguages).toEqual(["original", "zh", "en", "ja"]);
    expect(capabilitiesForProfile({ ...profile, textTranslation: "deepLX" }).targetLanguages).toEqual(["zh", "en", "ja"]);
    expect(capabilitiesForProfile({ ...profile, textTranslation: "openAICompatible" }).sourceLanguages).toEqual(["auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES]);
    expect(capabilitiesForProfile({ ...profile, textTranslation: "openAICompatible" }).targetLanguages).toEqual(["original", ...QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES]);
    expect(capabilitiesForProfile({ ...profile, textTranslation: "followService" }).targetLanguages).toHaveLength(32);
  }
});

const NATIVE_CAPABILITIES = {
  profileId: "ali", provider: "alibabaCloud", textTranslation: "followService", targetLanguage: "zh",
  sourceLanguages: ["auto", "fr"], targetLanguages: ["original", "zh", "fr"],
} as const;

it("uses actual stamped native options instead of an older local fallback", () => {
  const settings = { ...BASE_SETTINGS, languageCapabilities: NATIVE_CAPABILITIES };
  expect(sourceLanguagesForSettings(settings)).toEqual(["auto", "fr"]);
  expect(targetLanguagesForSettings(settings)).toEqual(["original", "zh", "fr"]);
  expect(translationModesForSettings(settings)).toEqual(["turbo"]);
});

it.each([
  { profileId: "old-profile" },
  { provider: "deepLX" as const },
  { textTranslation: "deepL" as const },
  { targetLanguage: "original" as const },
])("rejects native options with an expired %j stamp", (changedStamp) => {
  const settings = { ...BASE_SETTINGS, languageCapabilities: { ...NATIVE_CAPABILITIES, ...changedStamp } };
  expect(sourceLanguagesForSettings(settings)).toEqual(sourceLanguagesForSettings(BASE_SETTINGS));
  expect(targetLanguagesForSettings(settings)).toEqual(targetLanguagesForSettings(BASE_SETTINGS));
});

it("does not retain a matching-target snapshot across a saved route change", () => {
  const settings = { ...BASE_SETTINGS,
    profiles: [{ ...BASE_SETTINGS.profiles[0], textTranslation: "deepLX" as const }],
    languageCapabilities: NATIVE_CAPABILITIES,
  };
  expect(sourceLanguagesForSettings(settings)).toEqual(["auto", "ja", "en", "ko", "zh"]);
  expect(targetLanguagesForSettings(settings)).toEqual(["zh", "en", "ja"]);
});

it("falls back atomically for malformed native options and deduplicates valid lists", () => {
  for (const broken of [
    { sourceLanguages: [] }, { targetLanguages: [] },
    { sourceLanguages: ["unverified"] }, { targetLanguages: ["unverified"] },
  ]) {
    const settings = { ...BASE_SETTINGS, languageCapabilities: { ...NATIVE_CAPABILITIES, ...broken } } as SettingsSnapshot;
    expect(sourceLanguagesForSettings(settings)).toEqual(sourceLanguagesForSettings(BASE_SETTINGS));
    expect(targetLanguagesForSettings(settings)).toEqual(targetLanguagesForSettings(BASE_SETTINGS));
  }
  const settings = { ...BASE_SETTINGS, languageCapabilities: { ...NATIVE_CAPABILITIES, sourceLanguages: ["auto", "fr", "fr"] as const } };
  expect(sourceLanguagesForSettings(settings)).toEqual(["auto", "fr"]);
});

it("preserves an explicit Alibaba translation target when choosing Chinese", () => {
  const settings = { ...BASE_SETTINGS, sourceLanguage: "auto" as const, targetLanguage: "en" as const };
  expect(targetLanguageAfterSourceSwitch(settings, "zh")).toBe("en");
});

it("keeps Original when choosing a supported recognition language after Chinese", () => {
  const settings = { ...BASE_SETTINGS, sourceLanguage: "zh" as const, targetLanguage: "original" as const };
  expect(sourceLanguagesForSettings(settings)).toContain("no");
  expect(targetLanguageAfterSourceSwitch(settings, "no")).toBe("original");
});

it("keeps the selectable language catalogs aligned with the Rust realtime models", () => {
  expect(audio3).toMatch(/pub const MODEL:\s*&'static str\s*=\s*"qwen-audio-3\.0-asr-flash-streaming";/);
  expect(qwenMt).toMatch(/pub const REALTIME_MT_MODEL:\s*QwenMTModel\s*=\s*QwenMTModel::Lite;/);
  const table = qwenMt.match(/pub const QWEN_MT_LITE_LANGUAGE_CODES:[\s\S]*?=\s*&\[([\s\S]*?)\];/)?.[1];
  expect(table).toBeDefined();
  expect(Array.from(table!.matchAll(/"([a-z_]+)"/g), match => match[1])).toEqual(QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES);
});

it("uses the same official language catalogs as the Rust and Android encoders", () => {
  for (const entry of contract.speechLanguageCatalogs) {
    const provider = SERVICE_PROVIDERS.find(provider => provider === entry.provider)!;
    const capabilities = capabilitiesForProvider(provider);
    expect(capabilities.sourceLanguages, entry.id).toEqual(entry.expected.sourceLanguages);
    expect(capabilities.targetLanguages, entry.id).toEqual(entry.expected.targetLanguages);
  }
});
it.each(["customDashScopeASR", "customOpenAIASR"] as const)("exposes protocol codes for %s without bypassing the independent text encoder", provider => {
  const profile = { id: "custom", name: "Custom", provider, credentialState: "present", textTranslation: "deepL" } as const;
  expect(capabilitiesForProfile(profile, "original").sourceLanguages).toEqual(["auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES]);
  expect(capabilitiesForProfile(profile, "zh").sourceLanguages).not.toContain("fr");
});


it.each(["customDashScopeASR", "customOpenAIASR"] as const)("filters %s by declaration and text encoder without inferring model support", provider => {
  const profile = { id: "custom", name: "Any model", provider, credentialState: "present" as const, customSpeechSourceLanguages: ["en", "fr"] as const };
  const declared = { ...profile, customSpeechSourceLanguages: [...profile.customSpeechSourceLanguages] };
  for (const textTranslation of ["openAICompatible", "chatMock"] as const) {
    expect(capabilitiesForProfile({ ...declared, textTranslation }).sourceLanguages).toEqual(["auto", "en", "fr"]);
    expect(capabilitiesForProfile({ ...declared, textTranslation }).targetLanguages).toEqual(["original", ...QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES]);
  }
  expect(capabilitiesForProfile({ ...declared, textTranslation: "deepL" }).sourceLanguages).toEqual(["auto", "en"]);
  expect(capabilitiesForProfile({ ...declared, customSpeechSourceLanguages: [] }).sourceLanguages).toEqual(["auto"]);
});

it.each([undefined, null, ["en", "fr"] as const])("does not let a stale native custom list hide expanded or cleared declarations: %j", declaration => {
  const profile = { id: "custom", name: "Custom", provider: "customDashScopeASR" as const, credentialState: "present" as const, textTranslation: "openAICompatible" as const,
    customSpeechSourceLanguages: declaration ? [...declaration] : declaration };
  const snapshot: SettingsSnapshot = { ...BASE_SETTINGS, activeProfileId: profile.id, profiles: [profile], languageCapabilities: {
    profileId: profile.id, provider: profile.provider, textTranslation: profile.textTranslation, targetLanguage: "zh",
    sourceLanguages: ["auto", "en"], targetLanguages: ["original", "zh", "en", "ja"],
  } };
  expect(sourceLanguagesForSettings(snapshot)).toEqual(declaration ? ["auto", "en", "fr"] : ["auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES]);
  expect(targetLanguagesForSettings(snapshot)).toEqual(["original", ...QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES]);
});
