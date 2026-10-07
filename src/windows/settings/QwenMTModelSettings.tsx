import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import type { QwenMTModel, ServiceProfile } from "../../lib/types";
import { SettingsSelect } from "./SettingsPrimitives";
import { SettingsHelp } from "./SettingsHelp";
import { useSettingsToast } from "./useSettingsToast";

const copy = {
  zh: { lite: "低延迟·流式", flash: "均衡·流式", plus: "质量优先·整段", label: "翻译模型", help: "Lite、Flash 支持流式出字；Plus 整段返回。停止字幕后可切换。", failed: "翻译模型保存失败，请重试。" },
  "zh-TW": { lite: "低延遲·串流", flash: "均衡·串流", plus: "品質優先·整段", label: "翻譯模型", help: "Lite、Flash 支援串流出字；Plus 整段傳回。停止字幕後可切換。", failed: "翻譯模型儲存失敗，請重試。" },
  en: { lite: "fast · streaming", flash: "balanced · streaming", plus: "quality · full text", label: "Translation model", help: "Lite and Flash stream text; Plus returns a complete translation. Stop subtitles to switch models.", failed: "Could not save the translation model. Try again." },
  ja: { lite: "低遅延・逐次", flash: "バランス・逐次", plus: "高品質・一括", label: "翻訳モデル", help: "Lite と Flash は逐次表示、Plus は翻訳全体を返します。字幕を停止して切り替えてください。", failed: "翻訳モデルを保存できませんでした。再試行してください。" },
  de: { lite: "schnell · Stream", flash: "ausgewogen · Stream", plus: "Qualität · komplett", label: "Übersetzungsmodell", help: "Lite und Flash liefern Text schrittweise; Plus liefert die vollständige Übersetzung. Zum Wechseln Untertitel stoppen.", failed: "Übersetzungsmodell konnte nicht gespeichert werden. Bitte erneut versuchen." },
  ko: { lite: "저지연·스트리밍", flash: "균형·스트리밍", plus: "고품질·전체", label: "번역 모델", help: "Lite와 Flash는 텍스트를 순차적으로 표시하고 Plus는 전체 번역을 반환합니다. 자막을 중지한 후 변경하세요.", failed: "번역 모델을 저장하지 못했습니다. 다시 시도하세요." },
  fr: { lite: "rapide · flux", flash: "équilibré · flux", plus: "qualité · texte entier", label: "Modèle de traduction", help: "Lite et Flash affichent le texte progressivement ; Plus renvoie une traduction complète. Arrêtez les sous-titres pour changer de modèle.", failed: "Impossible d’enregistrer le modèle de traduction. Réessayez." },
};

export function QwenMTModelSettings({ profile, disabled, onSave }: {
  profile: ServiceProfile;
  disabled: boolean;
  onSave: (model: QwenMTModel) => Promise<unknown>;
}) {
  const strings = copy[effectiveUiLanguage()];
  const { runWithToast } = useSettingsToast();
  return <div className="settings-field service-stage__selector">
    <span className="service-stage__name-help">{strings.label}<SettingsHelp text={strings.help} label={I18N.settings.helpLabel} /></span>
    <SettingsSelect label={strings.label} value={profile.qwenMtModel ?? "lite"} disabled={disabled}
      options={(["lite", "flash", "plus"] as const).map(value => ({ value, label: `Qwen-MT ${value[0].toUpperCase()}${value.slice(1)}（${strings[value]}）` }))}
      onChange={value => { if (value !== (profile.qwenMtModel ?? "lite")) void runWithToast(() => onSave(value as QwenMTModel), strings.failed); }} />
  </div>;
}
