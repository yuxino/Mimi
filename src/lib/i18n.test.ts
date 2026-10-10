import { afterEach, describe, expect, it, vi } from "vitest";
import { effectiveUiLanguage, I18N, UI_LANGUAGES, UI_LANGUAGE_OPTIONS, setStoredUiLanguage, subscribeUiLanguage } from "./i18n";
import { SUBTITLE_DISPLAY_OPTIONS } from "./subtitleDisplay";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES } from "./types";

afterEach(() => { setStoredUiLanguage("en"); vi.unstubAllGlobals(); });

describe("in-place language changes", () => {
  it("updates copy and display tables without reloading the window", () => {
    const reload = vi.fn();
    vi.stubGlobal("window", { location: { reload } });
    setStoredUiLanguage("en");
    const listener = vi.fn();
    const unsubscribe = subscribeUiLanguage(listener);
    const english = I18N.settings.installUpdate;
    setStoredUiLanguage("zh");
    expect(effectiveUiLanguage()).toBe("zh");
    expect(I18N.settings.installUpdate).not.toBe(english);
    expect(SOURCE_LANGUAGE_DISPLAY_NAMES.en).toBe("英语");
    expect(SUBTITLE_DISPLAY_OPTIONS[0].label).toBe(I18N.settings.displayTranslation);
    expect(TARGET_LANGUAGE_DISPLAY_NAMES.zh).toBe("简体中文");
    expect(listener).toHaveBeenCalledTimes(1);
    setStoredUiLanguage("zh");
    expect(listener).toHaveBeenCalledTimes(1);
    setStoredUiLanguage("ja");
    expect(SOURCE_LANGUAGE_DISPLAY_NAMES.en).toBe("英語");
    expect(SUBTITLE_DISPLAY_OPTIONS[0].label).toBe(I18N.settings.displayTranslation);
    expect(reload).not.toHaveBeenCalled();
    unsubscribe();
  });

  it("notifies a window even when another window already changed shared storage", () => {
    setStoredUiLanguage("en");
    vi.stubGlobal("localStorage", { getItem: () => "ja", setItem: vi.fn() });
    const listener = vi.fn();
    const unsubscribe = subscribeUiLanguage(listener);
    setStoredUiLanguage("ja");
    expect(listener).toHaveBeenCalledOnce();
    expect(effectiveUiLanguage()).toBe("ja");
    unsubscribe();
  });
});

it("keeps the text destination labels short in all UI languages", () => {
  for (const language of UI_LANGUAGES) {
    setStoredUiLanguage(language);
    expect(I18N.settings.deepLXChain).toContain("DeepLX");
    expect(I18N.settings.deepLXChain.length).toBeLessThan(55);
    expect(I18N.settings.deepLXToken).not.toContain("Bearer");
    expect(I18N.settings.savedCredential).not.toBe("");
    expect(I18N.settings.asrApiKey).not.toBe(I18N.settings.deepLXEndpoint);
    expect(I18N.settings.textTranslationCustom).toBe("DeepLX");
    expect(I18N.settings.openAICompatibleRequirements).toContain("POST /chat/completions");
    expect(I18N.settings.openAICompatibleRequirements).toContain("model/messages");
    expect(I18N.settings.openAICompatibleRequirements).toContain("choices[0].message.content");
    expect(I18N.settings.openAICompatibleRequired).not.toBe("");
    expect(I18N.settings.openAICompatibleAddressKey).not.toBe("");
    expect(I18N.settings.textTranslationOpenAICompatible).not.toContain("ChatMock");
    expect(I18N.settings.chatMockSetup).toContain("/v1/models");
    expect(I18N.settings.chatMockModelPlaceholder).toContain("ChatMock");
    expect(I18N.settings.chatMockSetup).toContain("http://127.0.0.1:8000/v1");
    expect(I18N.settings.chatMockSetup).toContain("ChatGPT");
    expect(I18N.settings.optionalTranslationKeyPlaceholder).not.toBe("");
    expect(I18N.settings.noTranslationKeyPlaceholder).not.toBe("");
    expect(I18N.settings.removeTranslationApiKey).not.toBe(I18N.settings.cancelTranslationKeyRemoval);
  }
});

it("explains the recording scope and explicit source change in all UI languages", () => {
  for (const language of ["zh", "en", "ja"] as const) {
    setStoredUiLanguage(language);
    expect(I18N.settings.audioInputSystem).not.toBe(I18N.settings.audioInputMicrophone);
    expect(I18N.settings.recordSessionAudioHelp).toContain({ en: "microphone", zh: "麦克风", ja: "マイク" }[language]);
    expect(I18N.settings.audioInputHelp).toContain({ en: "recording off", zh: "关闭录音", ja: "録音はオフ" }[language]);
    expect(I18N.settings.sessionMicrophoneEnabled).toContain(I18N.settings.audioInputMicrophone);
  }
});

const localizedExamples = {
  "zh-TW": { english: "英文" },
  de: { english: "Englisch" },
  fr: { english: "anglais" },
  ko: { english: "영어" },
  th: { english: "อังกฤษ" },
} as const;

it.each(["zh-TW", "de", "fr", "ko", "th"] as const)("switches every surface and language labels to %s without reload", language => {
  setStoredUiLanguage("en");
  const english = { tray: I18N.tray.settings, settings: I18N.settings.applicationTitle, overlay: I18N.overlay.phaseIdle, modes: I18N.modes.lowLatencyHelp };
  const listener = vi.fn();
  const unsubscribe = subscribeUiLanguage(listener);
  setStoredUiLanguage(language);
  expect(effectiveUiLanguage()).toBe(language);
  expect(I18N.tray.settings).not.toBe(english.tray);
  expect(I18N.settings.applicationTitle).not.toBe(english.settings);
  expect(I18N.overlay.phaseIdle).not.toBe(english.overlay);
  expect(I18N.modes.lowLatencyHelp).not.toBe(english.modes);
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.en.toLocaleLowerCase()).toBe(localizedExamples[language].english.toLocaleLowerCase());
  expect(TARGET_LANGUAGE_DISPLAY_NAMES.zh).not.toBe(TARGET_LANGUAGE_DISPLAY_NAMES.zh_tw);
  expect(listener).toHaveBeenCalledOnce();
  unsubscribe();
});

it.each([
  ["zh-TW", "zh-TW"], ["zh-HK", "zh-TW"], ["zh-MO", "zh-TW"], ["zh-Hant", "zh-TW"], ["zh_Hant_CN", "zh-TW"], ["zh-Hans-HK", "zh"], ["zh-CN", "zh"], ["zh-SG", "zh"], ["zh", "zh"],
  ["de-DE", "de"], ["DE_at", "de"], ["fr-CA", "fr"], ["ko-KR", "ko"], ["th-TH", "th"], ["TH_th", "th"], ["es-ES", "en"],
] as const)("resolves system locale %s to %s and honors the override", (system, expected) => {
  vi.stubGlobal("navigator", { language: system });
  setStoredUiLanguage("system");
  expect(effectiveUiLanguage()).toBe(expected);
  setStoredUiLanguage("ja");
  expect(effectiveUiLanguage()).toBe("ja");
});

it("keeps language choices in their own native spelling", () => {
  expect(UI_LANGUAGE_OPTIONS.map(option => option.label)).toEqual(["简体中文", "繁體中文", "English", "日本語", "Deutsch", "한국어", "Français", "ภาษาไทย"]);
});

it("keeps all primary locale keys, functions and values complete", () => {
  setStoredUiLanguage("en");
  const groups = ["tray", "overlay", "settings", "modes"] as const;
  const baseline = Object.fromEntries(groups.map(group => [group, I18N[group]]));
  for (const language of UI_LANGUAGES) {
    setStoredUiLanguage(language);
    for (const group of groups) {
      expect(Object.keys(I18N[group]).sort()).toEqual(Object.keys(baseline[group]).sort());
      for (const [key, value] of Object.entries(I18N[group])) {
        expect(typeof value, `${language}.${group}.${key}`).toBe(typeof baseline[group][key as keyof typeof baseline[typeof group]]);
        expect(typeof value === "function" ? Reflect.apply(value, undefined, [1, "English", "English"]) : value, `${language}.${group}.${key}`).not.toBe("");
      }
    }
  }
});
