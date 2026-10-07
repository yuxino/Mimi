import { useEffect, useRef, useState } from "react";
import { I18N } from "../../lib/i18n";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type ProfileLanguagePreset, type ServiceProfile, type SettingsSnapshot } from "../../lib/types";
import { InlineFeedback, SettingsRow } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

/** Capture the current pair explicitly; temporary language changes never write it. */
export function ProfileLanguagePresetSettings({ profile, settings, disabled, onSave }: {
  profile: ServiceProfile;
  settings: SettingsSnapshot;
  disabled: boolean;
  onSave: (preset: ProfileLanguagePreset | null) => Promise<SettingsSnapshot>;
}) {
  const saved = profile.languagePreset ?? null;
  const current = { sourceLanguage: settings.sourceLanguage, targetLanguage: settings.targetLanguage };
  const active = profile.id === settings.activeProfileId;
  const matches = saved?.sourceLanguage === current.sourceLanguage && saved?.targetLanguage === current.targetLanguage;
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState<{ preset: ProfileLanguagePreset | null } | null>(null);
  const pending = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const { beginToast } = useSettingsToast();
  const locked = disabled || saving;
  const save = async (preset: ProfileLanguagePreset | null) => {
    if (locked || pending.current || (preset && !active)) return;
    pending.current = true; setSaving(true); setFailed(null);
    const notify = beginToast();
    try {
      const after = await onSave(preset);
      const acknowledged = after.profiles.find(item => item.id === profile.id)?.languagePreset ?? null;
      if (acknowledged?.sourceLanguage !== preset?.sourceLanguage || acknowledged?.targetLanguage !== preset?.targetLanguage) throw new Error("profile_language_preset_not_saved");
      if (mounted.current) notify(preset ? I18N.settings.profileLanguagesSaved : I18N.settings.profileLanguagesCleared);
    } catch {
      if (mounted.current) setFailed({ preset });
    } finally {
      pending.current = false;
      if (mounted.current) setSaving(false);
    }
  };
  return <div className="profile-language-preset" aria-busy={saving}>
    <SettingsRow label={I18N.settings.profileLanguagesTitle} description={active ? I18N.settings.profileLanguagesHelp : I18N.settings.profileLanguagesInactiveHelp}
      feedback={failed && <InlineFeedback tone="error">{I18N.settings.profileLanguagesSaveFailed}<button type="button" className="settings-link" disabled={locked} onClick={() => void save(failed.preset)}>{I18N.settings.retryLoadingSettings}</button></InlineFeedback>}>
      <span className="profile-language-preset__controls">
        {saved && <span>{SOURCE_LANGUAGE_DISPLAY_NAMES[saved.sourceLanguage]} → {TARGET_LANGUAGE_DISPLAY_NAMES[saved.targetLanguage]}</span>}
        {active && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={locked || matches} onClick={() => void save(current)}>
          {saved ? matches ? I18N.settings.profileLanguagesRemembered : I18N.settings.profileLanguagesUpdate : I18N.settings.profileLanguagesRemember}
        </button>}
        {saved ? <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={locked} onClick={() => void save(null)}>{I18N.settings.profileLanguagesClear}</button>
          : !active && <span>{I18N.settings.profileLanguagesKeep}</span>}
      </span>
    </SettingsRow>
  </div>;
}
