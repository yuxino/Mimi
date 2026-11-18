import { useRef, useState } from "react";
import { ProviderIcon } from "../../../components/providers/ProviderIcon";
import { I18N } from "../../../lib/i18n";
import { profileErrorMessage } from "../../../lib/providers/connectionDiagnostics";
import { profileSelectionState } from "../../../lib/profiles/profileSelection";
import { useStore } from "../../../lib/store";
import { AudioInputSettings } from "./AudioInputSettings";
import { ProfileLanguageSettings } from "../services/languages/ProfileLanguageSettings";
import { SettingsRow, SettingsSection, SettingsSelect } from "../controls/SettingsPrimitives";
import { settingsPageCopy } from "../navigation/SettingsPageCopy";
import { useSettingsToast } from "../feedback/useSettingsToast";

export function VoiceSettings({ onOpenModels, onOpenAppleResources }: { onOpenModels: () => void; onOpenAppleResources: () => void }) {
  const settings = useStore(state => state.settings);
  const status = useStore(state => state.session.status.kind);
  const selectProfile = useStore(state => state.selectProfile);
  const [selectingProfile, setSelectingProfile] = useState(false);
  const inFlight = useRef(false);
  const { beginToast } = useSettingsToast();
  const busy = selectingProfile || status === "connecting" || status === "stopping";
  const choose = async (id: string) => {
    if (busy || inFlight.current || id === settings.activeProfileId) return;
    const profile = settings.profiles.find(profile => profile.id === id);
    if (!profile || profileSelectionState(profile, settings.targetLanguage).disabled) return;
    inFlight.current = true; setSelectingProfile(true);
    const notify = beginToast();
    try { await selectProfile(id); }
    catch (error) { notify(profileErrorMessage(error), true); }
    finally { inFlight.current = false; setSelectingProfile(false); }
  };
  return <div className="voice-settings">
    <AudioInputSettings />
    <SettingsSection id="voice-model" title={settingsPageCopy().selection} hideHeading>
      <SettingsRow label={settingsPageCopy().selection}>
        {settings.profiles.length ? <SettingsSelect label={settingsPageCopy().selection} value={settings.activeProfileId} busy={busy}
          options={settings.profiles.map(profile => ({ value: profile.id, label: profile.name, ...profileSelectionState(profile, settings.targetLanguage),
            icon: <ProviderIcon provider={profile.provider === "deepLX" ? "alibabaCloud" : profile.provider} size={32} /> }))}
          onChange={id => { void choose(id); }} /> : <button type="button" className="settings-button settings-button--quiet settings-button--compact" onClick={onOpenModels}>{I18N.settings.addProfile}</button>}
      </SettingsRow>
    </SettingsSection>
    {settings.profiles.some(profile => profile.id === settings.activeProfileId) && <ProfileLanguageSettings settings={settings} busy={busy} onOpenAppleResources={onOpenAppleResources} />}
  </div>;
}
