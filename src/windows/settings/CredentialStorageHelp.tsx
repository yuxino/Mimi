import { I18N } from "../../lib/i18n";
import { diagnosticCopy } from "../../lib/connectionDiagnostics";
import type { ServiceProfile } from "../../lib/types";
import { SettingsHelp } from "./SettingsHelp";

export function CredentialStorageHelp({ profile, id, readOnly = false }: {
  profile: ServiceProfile;
  id: string;
  readOnly?: boolean;
}) {
  const text = readOnly
    ? profile.credentialState === "unavailable" ? diagnosticCopy().localDevUnavailable : diagnosticCopy().localDevReadOnly
    : I18N.settings.credentialNote;
  return <span className="credential-storage-help">
    <span>{I18N.settings.credentials}</span>
    <SettingsHelp id={id} text={text} label={`${I18N.settings.credentials}: ${I18N.settings.helpLabel}`} />
  </span>;
}
