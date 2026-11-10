import { useId, type ReactNode } from "react";
import { effectiveUiLanguage } from "../../lib/i18n";

const copy = {
  zh: { models: "模型", setup: "服务设置", languages: "语言预设", management: "配置管理" },
  "zh-TW": { models: "模型", setup: "服務設定", languages: "語言預設", management: "設定管理" },
  en: { models: "Models", setup: "Service setup", languages: "Language preset", management: "Configuration" },
  ja: { models: "モデル", setup: "サービス設定", languages: "言語プリセット", management: "設定の管理" },
  ko: { models: "모델", setup: "서비스 설정", languages: "언어 프리셋", management: "구성 관리" },
  de: { models: "Modelle", setup: "Diensteinrichtung", languages: "Sprachvorlage", management: "Konfiguration" },
  fr: { models: "Modèles", setup: "Configuration du service", languages: "Préréglage de langues", management: "Gestion de la configuration" },
  th: { models: "โมเดล", setup: "ตั้งค่าบริการ", languages: "ภาษาที่ตั้งไว้", management: "จัดการการตั้งค่า" },
  ru: { models: "Модели", setup: "Настройка сервиса", languages: "Языковая пара", management: "Управление конфигурацией" },
};

export function ServiceEditorModule({ kind, children }: { kind: keyof typeof copy.zh; children: ReactNode }) {
  const titleId = useId();
  return <section className={`service-editor-module service-editor-module--${kind}`} aria-labelledby={kind === "setup" ? undefined : titleId} aria-label={kind === "setup" ? copy[effectiveUiLanguage()][kind] : undefined}>
    {kind !== "setup" && <header className="service-editor-module__heading"><h3 id={titleId}>{copy[effectiveUiLanguage()][kind]}</h3></header>}
    {children}
  </section>;
}
