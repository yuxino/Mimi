import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { openTencentSetupPage, type TencentSetupPage } from "../../lib/ipc";
import { SettingsHelp } from "./SettingsHelp";
import { useSettingsToast } from "./useSettingsToast";

/** These actions open only the fixed public console pages in the native allowlist. */
export function TencentSetupHelp() {
  const { runWithToast } = useSettingsToast();
  const pages: { page: TencentSetupPage; label: string }[] = [
    { page: "account", label: I18N.settings.tencentGetAppId },
    { page: "apiKey", label: I18N.settings.tencentGetSecretPair },
    { page: "asr", label: I18N.settings.tencentEnableAsr },
  ];
  return <div className="tencent-setup-help">
    <span className="tencent-setup-help__label">{I18N.settings.tencentSetupTitle}<SettingsHelp
      text={I18N.settings.tencentSetupHelp} label={I18N.settings.tencentSetupTitle} /></span>
    <span className="tencent-setup-help__actions">{pages.map(({ page, label }) => <button
      key={page} type="button" className="settings-button settings-button--quiet settings-button--compact"
      onClick={() => { void runWithToast(() => openTencentSetupPage(page), I18N.settings.tencentSetupOpenFailed); }}
    >{label}<Icon name="chevron-right" /></button>)}</span>
  </div>;
}
