/**
 * UI copy grouped by window. Tray strings, language/mode display names, and
 * the overlay copy follow the effective UI language. Overlay activity-phase
 * labels live in `types.ts` beside their visual parameters.
 */

import * as DE from "./locales/de";
import * as EN from "./locales/en";
import * as FR from "./locales/fr";
import * as JA from "./locales/ja";
import * as KO from "./locales/ko";
import * as RU from "./locales/ru";
import * as TH from "./locales/th";
import * as ZH from "./locales/zh";
import * as ZH_TW from "./locales/zh-TW";

import type { ServiceProvider, UiLanguage } from "./types";

export type { UiLanguage } from "./types";
export type ResolvedUiLanguage = Exclude<UiLanguage, "system">;
export const UI_LANGUAGES = ["zh", "zh-TW", "en", "ja", "de", "ko", "fr", "th", "ru"] as const;
export const UI_LANGUAGE_OPTIONS = [
  { value: "zh", label: "简体中文" }, { value: "zh-TW", label: "繁體中文" },
  { value: "en", label: "English" },
  { value: "ja", label: "日本語" }, { value: "de", label: "Deutsch" },
  { value: "ko", label: "한국어" }, { value: "fr", label: "Français" },
  { value: "th", label: "ภาษาไทย" }, { value: "ru", label: "Русский" },
] as const;

function isUiLanguage(value: string | null | undefined): value is ResolvedUiLanguage {
  return UI_LANGUAGES.some(language => language === value);
}

const UI_LANGUAGE_STORAGE_KEY = "mimi-ui-language";

/** Returns the user-selected UI language override, if any. */
function getStoredUiLanguage(): UiLanguage | null {
  try {
    const value = localStorage.getItem(UI_LANGUAGE_STORAGE_KEY);
    return isUiLanguage(value) || value === "system"
      ? value
      : null;
  } catch {
    return null;
  }
}

let selectedUiLanguage = getStoredUiLanguage();
const languageListeners = new Set<() => void>();

export function subscribeUiLanguage(listener: () => void): () => void {
  languageListeners.add(listener);
  return () => {
    languageListeners.delete(listener);
  };
}

/** Resolves immutable, locale-only labels without rebuilding a whole record
 * for every property read. The cache is bounded by the UI language catalog. */
export function localizedRecord<T extends object>(read: () => T): T {
  const records = new Map<ResolvedUiLanguage, T>();
  const current = () => {
    const language = effectiveUiLanguage();
    let record = records.get(language);
    if (record === undefined) {
      record = read();
      records.set(language, record);
    }
    return record;
  };
  return new Proxy(current(), { get: (_target, key) => Reflect.get(current(), key) });
}

/** Applies a language override in this window and persists it for future windows. */
export function setStoredUiLanguage(language: UiLanguage): void {
  const previous = effectiveUiLanguage();
  selectedUiLanguage = language;
  try {
    localStorage.setItem(UI_LANGUAGE_STORAGE_KEY, language);
  } catch {
    // Storage may be unavailable; the backend preference still persists.
  }
  if (previous !== effectiveUiLanguage()) {
    languageListeners.forEach((listener) => listener());
  }
}

/** The effective UI language, honoring a stored override over the OS/webview
 * language. */
export function effectiveUiLanguage(): ResolvedUiLanguage {
  if (isUiLanguage(selectedUiLanguage)) return selectedUiLanguage;
  const system = typeof navigator !== "undefined" ? navigator.language ?? "" : "";
  const parts = system.toLowerCase().split(/[-_]/);
  const base = parts[0];
  if (base === "zh") {
    if (parts.includes("hant")) return "zh-TW";
    if (parts.includes("hans")) return "zh";
    if (parts.some(part => ["tw", "hk", "mo"].includes(part))) return "zh-TW";
  }
  return isUiLanguage(base) ? base : "en";
}

export type SettingsCopy = {
  [Key in keyof typeof ZH.SETTINGS]: (typeof ZH.SETTINGS)[Key] extends (
    ...args: infer Args
  ) => string
    ? (...args: Args) => string
    : string;
};

const COPY = {
  "zh-TW": { overlay: ZH_TW.OVERLAY, tray: ZH_TW.TRAY, settings: ZH_TW.SETTINGS, modes: ZH_TW.MODES, provider: ZH_TW.PROVIDER_COPY },
  zh: { overlay: ZH.OVERLAY, tray: ZH.TRAY, settings: ZH.SETTINGS, modes: ZH.MODES, provider: ZH.PROVIDER_COPY },
  en: { overlay: EN.OVERLAY, tray: EN.TRAY, settings: EN.SETTINGS, modes: EN.MODES, provider: EN.PROVIDER_COPY },
  ja: { overlay: JA.OVERLAY, tray: JA.TRAY, settings: JA.SETTINGS, modes: JA.MODES, provider: JA.PROVIDER_COPY },
  de: { overlay: DE.OVERLAY, tray: DE.TRAY, settings: DE.SETTINGS, modes: DE.MODES, provider: DE.PROVIDER_COPY },
  fr: { overlay: FR.OVERLAY, tray: FR.TRAY, settings: FR.SETTINGS, modes: FR.MODES, provider: FR.PROVIDER_COPY },
  ko: { overlay: KO.OVERLAY, tray: KO.TRAY, settings: KO.SETTINGS, modes: KO.MODES, provider: KO.PROVIDER_COPY },
  th: { overlay: TH.OVERLAY, tray: TH.TRAY, settings: TH.SETTINGS, modes: TH.MODES, provider: TH.PROVIDER_COPY },
  ru: { overlay: RU.OVERLAY, tray: RU.TRAY, settings: RU.SETTINGS, modes: RU.MODES, provider: RU.PROVIDER_COPY },
} satisfies Record<ResolvedUiLanguage, {
  overlay: typeof EN.OVERLAY; tray: typeof EN.TRAY; settings: SettingsCopy; modes: typeof EN.MODES;
  provider: { [Key in keyof typeof EN.PROVIDER_COPY]: string };
}>;

export const I18N = {
  get overlay() { return COPY[effectiveUiLanguage()].overlay; },
  get tray() { return COPY[effectiveUiLanguage()].tray; },
  get settings() { return COPY[effectiveUiLanguage()].settings; },
  get modes() { return COPY[effectiveUiLanguage()].modes; },
  get provider() { return COPY[effectiveUiLanguage()].provider; },
} as const;

export function providerDisplayName(provider: ServiceProvider): string {
  switch (provider) {
    case "appleSpeech":
      return I18N.settings.appleSpeech;
    case "customDashScopeASR":
      return I18N.settings.customDashScopeASR;
    case "customOpenAIASR":
      return I18N.settings.customOpenAIASR;
    case "alibabaCloud":
      return I18N.settings.providerAlibaba;
    case "openAIRealtime":
      return I18N.settings.providerOpenAI;
    case "googleGeminiLive":
      return I18N.settings.providerGoogleGemini;
    case "azureOpenAIRealtime":
      return I18N.settings.providerAzureOpenAI;
    case "volcanoEngine":
      return I18N.settings.providerVolcanoEngine;
    case "tencentCloud":
      return I18N.settings.providerTencentCloud;
    case "baiduTranslate":
      return I18N.settings.providerBaiduTranslate;
    case "deepLX":
      return I18N.settings.providerAlibaba;
    case "xAIRealtime":
      return I18N.settings.providerXAI;
  }
}
