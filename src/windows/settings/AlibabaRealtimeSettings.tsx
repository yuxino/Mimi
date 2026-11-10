import { PROVIDER_COPY } from "../../lib/generated/providerCopy";
import { useRef, useState } from "react";
import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import type { AlibabaRealtimeMode, ServiceProfile } from "../../lib/types";
import { AlibabaSetupLink } from "./AlibabaSetupLink";
import { Icon } from "../../components/Icon";
import { ConfigInput } from "./ConfigInput";
import { SettingsHelp } from "./SettingsHelp";
import { InlineFeedback, SettingsSelect } from "./SettingsPrimitives";

const copy = {
  zh: { saved: "已保存。", save: "保存", failed: "保存失败，请检查业务空间 ID 后重试。" },
  "zh-TW": { saved: "已儲存。", save: "儲存", failed: "儲存失敗，請檢查業務空間 ID 後重試。" },
  en: { saved: "Saved.", save: "Save", failed: "Could not save. Check the workspace ID and retry." },
  ja: { saved: "保存しました。", save: "保存", failed: "保存できませんでした。ワークスペース ID を確認して再試行してください。" },
  de: { saved: "Gespeichert.", save: "Speichern", failed: "Speichern fehlgeschlagen. Workspace-ID prüfen und erneut versuchen." },
  ko: { saved: "저장했습니다.", save: "저장", failed: "저장하지 못했습니다. 워크스페이스 ID를 확인한 후 다시 시도하세요." },
  fr: { saved: "Enregistré.", save: "Enregistrer", failed: "Échec de l’enregistrement. Vérifiez l’ID d’espace et réessayez." },
  th: { saved: "บันทึกแล้ว", save: "บันทึก", failed: "บันทึกไม่ได้ ตรวจสอบ ID เวิร์กสเปซแล้วลองอีกครั้ง" },
  ru: { saved: "Сохранено.", save: "Сохранить", failed: "Не удалось сохранить. Проверьте ID рабочей области и повторите попытку." },
};

export function AlibabaRealtimeSettings({ profile, disabled, onSave }: {
  profile: ServiceProfile;
  disabled: boolean;
  onSave: (mode: AlibabaRealtimeMode, workspaceId: string, savedMessage: string) => Promise<unknown>;
}) {
  const language = effectiveUiLanguage();
  const common = PROVIDER_COPY[language];
  const strings = {
    ...copy[language],
    chainDescription: common.alibabaAsrMtDescription,
    liveTranslateDescription: common.alibabaLiveDescription,
    label: common.alibabaModeLabel,
    chain: common.alibabaAsrMtLabel,
    simultaneous: common.alibabaLiveLabel,
    workspace: common.alibabaWorkspaceLabel,
    help: common.alibabaModeHelp,
    workspaceHelp: common.alibabaWorkspaceHelp,
  };
  const savedMode: AlibabaRealtimeMode = profile.alibabaRealtimeMode === "liveTranslate" ? "liveTranslate" : "disabled";
  const savedWorkspace = profile.alibabaWorkspaceId ?? "";
  const [mode, setMode] = useState<AlibabaRealtimeMode>(savedMode);
  const [workspace, setWorkspace] = useState(savedWorkspace);
  const [error, setError] = useState(false);
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const dirty = mode !== savedMode || workspace.trim() !== savedWorkspace;
  const validWorkspace = workspace.trim().length <= 63 && /^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$/.test(workspace.trim());
  const save = async () => {
    if (disabled || inFlight.current || !dirty || (mode !== "disabled" && !validWorkspace)) return;
    inFlight.current = true;
    setBusy(true); setError(false);
    try { await onSave(mode, workspace.trim(), strings.saved); }
    catch { setError(true); }
    finally { inFlight.current = false; setBusy(false); }
  };
  return <section className="service-stage alibaba-realtime-settings">
    <div className="settings-field service-stage__selector">
      <span className="service-stage__field-label alibaba-setup-field-label">{strings.label}<SettingsHelp text={strings.help} label={I18N.settings.helpLabel} /></span>
      <SettingsSelect label={strings.label} value={mode} disabled={disabled || busy}
        options={[{ value: "liveTranslate", label: strings.simultaneous, description: strings.liveTranslateDescription }, { value: "disabled", label: strings.chain, description: strings.chainDescription }]}
        onChange={value => { setMode(value as AlibabaRealtimeMode); if (value === "disabled") setWorkspace(savedWorkspace); setError(false); }} />
    </div>
    {mode !== "disabled" && <div className="settings-field">
      <span className="service-stage__field-label alibaba-setup-field-label"><label htmlFor={`profile-workspace-${profile.id}`}>{strings.workspace}</label><SettingsHelp text={strings.workspaceHelp} label={I18N.settings.helpLabel} /></span>
      <div className="alibaba-setup-control"><ConfigInput id={`profile-workspace-${profile.id}`} type="text" autoComplete="off" spellCheck={false} required
        disabled={disabled || busy} value={workspace} onValueChange={value => { setWorkspace(value); setError(false); }} />
        <AlibabaSetupLink page="workspace" disabled={disabled || busy} />
      </div>
    </div>}
    {error && <InlineFeedback tone="error">{strings.failed}</InlineFeedback>}
    {dirty && <span className="credential-form__actions">
      <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled || busy}
        onClick={() => { setMode(savedMode); setWorkspace(savedWorkspace); setError(false); }}>{I18N.settings.cancel}</button>
      <button type="button" className="settings-button settings-button--primary settings-button--compact" disabled={disabled || busy || (mode !== "disabled" && !validWorkspace)}
        aria-busy={busy || undefined} onClick={() => void save()}><Icon name="checkmark" />{strings.save}</button>
    </span>}
  </section>;
}
