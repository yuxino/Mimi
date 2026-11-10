import { effectiveUiLanguage } from "../../lib/i18n";

const copy = {
  zh: { voice: "声音与语言", models: "模型与服务", selection: "使用的配置" },
  "zh-TW": { voice: "聲音與語言", models: "模型與服務", selection: "使用的設定" },
  en: { voice: "Audio & Languages", models: "Models & Services", selection: "Active configuration" },
  ja: { voice: "音声と言語", models: "モデルとサービス", selection: "使用する設定" },
  ko: { voice: "오디오 및 언어", models: "모델 및 서비스", selection: "사용할 구성" },
  de: { voice: "Audio & Sprachen", models: "Modelle & Dienste", selection: "Aktive Konfiguration" },
  fr: { voice: "Audio et langues", models: "Modèles et services", selection: "Configuration active" },
  th: { voice: "เสียงและภาษา", models: "โมเดลและบริการ", selection: "การตั้งค่าที่ใช้" },
  ru: { voice: "Звук и языки", models: "Модели и сервисы", selection: "Активная конфигурация" },
};
export function settingsPageCopy() { return copy[effectiveUiLanguage()]; }
