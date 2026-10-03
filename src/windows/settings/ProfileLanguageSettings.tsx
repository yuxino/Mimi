import { useEffect, useRef, useState } from "react";
import { Switch } from "../../components/Switch";
import { Icon } from "../../components/Icon";
import { LanguageSelect } from "../../components/LanguageSelect";
import { I18N } from "../../lib/i18n";
import { sourceLanguagesForSettings, targetLanguagesForSettings } from "../../lib/providerCapabilities";
import { useStore } from "../../lib/store";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type SettingsDraft, type SettingsSnapshot } from "../../lib/types";
import { SettingsHelp } from "./SettingsHelp";
import { InlineFeedback, SettingsRow } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

/** Explicit language preferences belong to the active service. */
export function ProfileLanguageSettings({ settings, disabled, requiresStop = false }: { settings: SettingsSnapshot; disabled: boolean; requiresStop?: boolean }) {
  const saveSettings = useStore(state => state.saveSettings);
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState<"failed" | null>(null);
  const { beginToast } = useSettingsToast();
  const inFlight = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const sources = sourceLanguagesForSettings(settings);
  const targets = targetLanguagesForSettings(settings);
  const skipped = settings.targetLanguage === "original";
  const previousTarget = useRef(settings.targetLanguage);
  useEffect(() => { if (settings.targetLanguage !== "original") previousTarget.current = settings.targetLanguage; }, [settings.targetLanguage]);
  const save = async (draft: SettingsDraft) => {
    if (disabled || inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setFeedback(null);
    const notify = beginToast();
    try {
      await saveSettings(draft);
      if (mounted.current) notify(I18N.settings.languageSaved);
    } catch {
      if (mounted.current) setFeedback("failed");
    } finally {
      inFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  return <section id="translation-languages" className="profile-language-settings" aria-labelledby="translation-languages-title" aria-busy={busy}>
    <header className="profile-language-settings__heading"><h3 id="translation-languages-title">{I18N.settings.subtitleLanguages}</h3>{requiresStop && <SettingsHelp text={I18N.settings.languageChangeRequiresStop} label={I18N.settings.helpLabel} icon="lock" />}</header>
    <SettingsRow label={I18N.settings.sourceLanguage} description={I18N.settings.recognitionLanguageHelp} align="start">
      <LanguageChoices label={I18N.settings.sourceLanguage} value={settings.sourceLanguage} disabled={disabled || busy || sources.length === 1}
        options={sources.map(value => ({ value, label: SOURCE_LANGUAGE_DISPLAY_NAMES[value] }))}
        onChange={value => { const sourceLanguage = sources.find(language => language === value); if (sourceLanguage) void save({ sourceLanguage }); }} />
    </SettingsRow>
    {targets.includes("original") && <SettingsRow label={I18N.settings.skipTranslation} description={I18N.settings.skipTranslationHelp}>
      <Switch aria-label={I18N.settings.skipTranslation} checked={skipped} disabled={disabled || busy}
        onChange={skip => {
          const previous = previousTarget.current;
          const targetLanguage = skip ? "original" : previous !== "original" && targets.includes(previous) ? previous : targets.find(target => target !== "original");
          if (targetLanguage) void save({ targetLanguage });
        }} />
    </SettingsRow>}
    <SettingsRow label={I18N.settings.translateTo} align="start">
      <LanguageChoices label={I18N.settings.translateTo} value={settings.targetLanguage} disabled={disabled || busy || skipped || targets.length === 1}
        options={targets.map(value => ({ value, label: TARGET_LANGUAGE_DISPLAY_NAMES[value] }))}
        onChange={value => { const targetLanguage = targets.find(language => language === value); if (targetLanguage) void save({ targetLanguage }); }} />
    </SettingsRow>
    {feedback && <InlineFeedback tone="error">{I18N.settings.languageSaveFailed}</InlineFeedback>}
  </section>;
}

function LanguageChoices({ label, value, options, disabled, onChange }: {
  label: string; value: string; options: readonly { value: string; label: string }[]; disabled: boolean; onChange: (value: string) => void;
}) {
  if (options.length > 6) return <LanguageSelect label={label} value={value} options={options} disabled={disabled} onChange={onChange} />;
  return <div className="profile-language-choices" role="group" aria-label={label}>
    {options.map(option => <button key={option.value} type="button" className="profile-language-choice" aria-pressed={option.value === value}
      disabled={disabled} onClick={() => { if (option.value !== value) onChange(option.value); }}>
      <span>{option.label}</span>{option.value === value && <Icon name="checkmark" />}
    </button>)}
  </div>;
}
