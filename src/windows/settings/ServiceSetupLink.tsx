import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { SERVICE_SETUPS } from "../../lib/generated/providerCopy";
import { openServiceSetupPage, type TextTranslationSetupPage } from "../../lib/ipc";
import { useSettingsToast } from "./useSettingsToast";

/** Fixed official service destinations, independent of editable API endpoints. */
export function ServiceSetupLink({ serviceId, page = "apiKey", disabled = false }: {
  serviceId: string;
  page?: TextTranslationSetupPage;
  disabled?: boolean;
}) {
  const service = SERVICE_SETUPS.find(item => item.id === serviceId);
  const copy = I18N.provider;
  const { runWithToast } = useSettingsToast();
  if (!service || !(page === "apiKey" ? service.apiKeyUrl : service.guideUrl)) return null;
  const label = page === "apiKey" ? copy.compatibleSetupGetKey : copy.compatibleSetupGuide;
  return <button type="button" className="settings-link translation-setup-link" disabled={disabled} aria-label={`${label}: ${service.label}`}
    onClick={() => { void runWithToast(() => openServiceSetupPage(service.id, page), copy.compatibleSetupOpenFailed); }}>
    {label}<Icon name="chevron-right" />
  </button>;
}
