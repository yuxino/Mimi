import { I18N } from "../../lib/i18n";
import type { ServiceProfile } from "../../lib/types";
import { SettingsHelp } from "./SettingsHelp";

export function CredentialStorageHelp({ profile, id }: {
  profile: ServiceProfile;
  id: string;
}) {
  const text = profile.credentialStorage === "localFile" ? I18N.settings.credentialLocalFileHelp : I18N.settings.credentialNote;
  return <span className="credential-storage-help">
    <span>{I18N.settings.credentials}</span>
    <SettingsHelp id={id} text={text} label={`${I18N.settings.credentials}: ${I18N.settings.helpLabel}`} />
  </span>;
}
