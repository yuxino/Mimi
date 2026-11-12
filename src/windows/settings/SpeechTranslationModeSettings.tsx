import { useRef, useState } from "react";
import { I18N } from "../../lib/i18n";
import type { ServiceProfile, SettingsSnapshot, SpeechTranslationMode } from "../../lib/types";
import { SettingsHelp } from "./SettingsHelp";
import { SettingsSelect } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

export function SpeechTranslationModeSettings({ profile, disabled, onSave }: {
  profile: ServiceProfile;
  disabled: boolean;
  onSave: (options: { speechTranslationMode: SpeechTranslationMode }) => Promise<SettingsSnapshot>;
}) {
  const copy = I18N.provider;
  const { runWithToast } = useSettingsToast();
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const mode = profile.speechTranslationMode ?? "simultaneous";
  const save = (next: SpeechTranslationMode) => {
    if (disabled || inFlight.current || next === mode) return;
    inFlight.current = true;
    setBusy(true);
    void runWithToast(async () => {
      try {
        const saved = await onSave({ speechTranslationMode: next });
        const acknowledged = saved.profiles.find(item => item.id === profile.id)?.speechTranslationMode ?? "simultaneous";
        if (acknowledged !== next) throw new Error("speech_translation_mode_not_saved");
      }
      finally { inFlight.current = false; setBusy(false); }
    }, copy.speechTranslationModeSaveFailed);
  };
  return <section className="service-stage speech-translation-mode-settings">
    <div className="settings-field service-stage__selector">
      <span className="service-stage__name-help">{copy.speechTranslationModeLabel}<SettingsHelp text={copy.speechTranslationModeHelp} label={I18N.settings.helpLabel} /></span>
      <SettingsSelect label={copy.speechTranslationModeLabel} value={mode} disabled={disabled || busy}
        options={[{ value: "simultaneous", label: copy.speechTranslationSimultaneousLabel }, { value: "asrMt", label: copy.speechTranslationAsrMtLabel }]}
        onChange={value => save(value as SpeechTranslationMode)} />
    </div>
  </section>;
}
