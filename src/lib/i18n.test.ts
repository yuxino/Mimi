import { afterEach, describe, expect, it, vi } from "vitest";
import { effectiveUiLanguage, I18N, setStoredUiLanguage, subscribeUiLanguage } from "./i18n";
import { SUBTITLE_DISPLAY_OPTIONS } from "./subtitleDisplay";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, TRANSLATION_MODE_DISPLAY_NAMES } from "./types";

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
    expect(TRANSLATION_MODE_DISPLAY_NAMES.turbo).toBe("极速");
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
  for (const language of ["zh", "en", "ja"] as const) {
    setStoredUiLanguage(language);
    expect(I18N.settings.deepLXChain).toContain("DeepLX");
    expect(I18N.settings.deepLXChain.length).toBeLessThan(55);
    expect(I18N.settings.deepLXToken).not.toContain("Bearer");
    expect(I18N.settings.savedServiceAddressPlaceholder).not.toBe("");
    expect(I18N.settings.asrApiKey).not.toBe(I18N.settings.deepLXEndpoint);
    expect(I18N.settings.textTranslationCustom).toBe("DeepLX");
    expect(I18N.settings.openAICompatibleRequirements).toContain("POST /chat/completions");
    expect(I18N.settings.openAICompatibleRequirements).toContain("model/messages");
    expect(I18N.settings.openAICompatibleRequirements).toContain("choices[0].message.content");
    expect(I18N.settings.openAICompatibleRequired).not.toBe("");
    expect(I18N.settings.openAICompatibleAddressKey).not.toBe("");
  }
});
