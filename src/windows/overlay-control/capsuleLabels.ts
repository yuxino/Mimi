import { SUPPLEMENTAL_EN } from "../../lib/locales/supplemental-schema";
import { supplemental } from "../../lib/locales/supplemental";
import { activeServiceProfile, isCustomSpeechProvider } from "../../lib/providerCapabilities";
import { effectiveUiLanguage } from "../../lib/i18n";
import { sourceLanguageDisplayName, targetLanguageDisplayName, type SettingsSnapshot } from "../../lib/types";

const labels = {
  en: SUPPLEMENTAL_EN.capsuleLabels_labels,
  zh: { bilingual: "中英互译", auto: "自动", serviceDefault: "服务默认", zh: "中文", en: "英语", ja: "日语", ko: "韩语", original: "原文", error: "错误", idle: "待机", paused: "暂停", translating: "翻译中", connecting: "连接中", stopping: "停止中" },
  ja: { bilingual: "中国語 ↔ 英語", auto: "自動", serviceDefault: "サービス既定", zh: "中国語", en: "英語", ja: "日本語", ko: "韓国語", original: "原文", error: "エラー", idle: "待機", paused: "一時停止", translating: "翻訳中", connecting: "接続中", stopping: "終了中" },
  ...supplemental.capsuleLabels_labels,
};

export function capsuleLabels(settings: Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage"> & Partial<Pick<SettingsSnapshot, "profiles" | "activeProfileId">>, phase: "error" | "idle" | "paused" | "translating" | "connecting" | "stopping" | null, language = effectiveUiLanguage()) {
  const text = labels[language];
  const profile = settings.profiles && settings.activeProfileId ? activeServiceProfile({ profiles: settings.profiles, activeProfileId: settings.activeProfileId }) : undefined;
  const bilingual = profile?.provider === "volcanoEngine";
  return {
    source: bilingual && settings.sourceLanguage === "zh_en" ? text.bilingual : settings.sourceLanguage === "auto" && settings.profiles && settings.activeProfileId &&
      isCustomSpeechProvider(activeServiceProfile({ profiles: settings.profiles, activeProfileId: settings.activeProfileId })?.provider ?? "alibabaCloud")
      ? text.serviceDefault : settings.sourceLanguage in text
      ? text[settings.sourceLanguage as keyof typeof text]
      : sourceLanguageDisplayName(settings.sourceLanguage, language),
    target: bilingual && settings.targetLanguage === "zh_en" ? text.bilingual : settings.targetLanguage in text
      ? text[settings.targetLanguage as keyof typeof text]
      : targetLanguageDisplayName(settings.targetLanguage, language),
    phase: phase ? text[phase] : null,
  };
}
