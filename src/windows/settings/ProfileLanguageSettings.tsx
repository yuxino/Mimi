import { useLanguageNormalizationToast } from "./useLanguageNormalizationToast";
import { useEffect, useRef, useState } from "react";
import { Switch } from "../../components/Switch";
import { Icon } from "../../components/Icon";
import { LanguageSelect } from "../../components/LanguageSelect";
import { speechLanguageGuidance } from "../../lib/speechLanguageGuidance";
import { I18N } from "../../lib/i18n";
import { languageActionErrorMessage } from "../../lib/connectionDiagnostics";
import { activeServiceProfile, sourceLanguagesForSettings, targetLanguagesForSettings } from "../../lib/providerCapabilities";
import { useStore } from "../../lib/store";
import { TARGET_LANGUAGE_DISPLAY_NAMES, type SettingsDraft, type SettingsSnapshot } from "../../lib/types";
import { SettingsHelp } from "./SettingsHelp";
import { SettingsRow } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

/** Explicit language preferences belong to the active service. */
export function ProfileLanguageSettings({ settings, disabled, requiresStop = false, onOpenAppleResources, hideSourceLanguage = false }: { settings: SettingsSnapshot; disabled: boolean; requiresStop?: boolean; onOpenAppleResources?: () => void; hideSourceLanguage?: boolean }) {
  const saveSettings = useStore(state => state.saveSettings);
  const [busy, setBusy] = useState(false);
  const { beginToast } = useSettingsToast();
  const trackLanguageChange = useLanguageNormalizationToast(settings);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const guidance = speechLanguageGuidance(settings);
  const sources = sourceLanguagesForSettings(settings);
  const appleSpeech = activeServiceProfile(settings)?.provider === "appleSpeech";
  const sourceNotice = sources.length === 0 && appleSpeech
    ? I18N.settings.appleSpeechNoReadyLanguages : guidance.notice;
  const targets = targetLanguagesForSettings(settings);
  const skipped = settings.targetLanguage === "original";
  const previousTarget = useRef(settings.targetLanguage);
  useEffect(() => { if (settings.targetLanguage !== "original") previousTarget.current = settings.targetLanguage; }, [settings.targetLanguage]);
  const save = async (draft: SettingsDraft) => {
    if (disabled || inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    const notify = beginToast();
    const finishLanguageChange = trackLanguageChange(draft.targetLanguage, notify);
    try {
      await saveSettings(draft);
      if (mounted.current) notify(I18N.settings.languageSaved);
      finishLanguageChange(true);
    } catch (error) {
      finishLanguageChange(false);
      if (mounted.current) notify(languageActionErrorMessage(error, I18N.settings.languageSaveFailed), true);
    } finally {
      inFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  if (hideSourceLanguage && targets.every(target => target === "original")) return null;
  return <section id="translation-languages" className="profile-language-settings" aria-labelledby="translation-languages-title" aria-busy={busy}>
    <header className="profile-language-settings__heading"><h3 id="translation-languages-title">{I18N.settings.subtitleLanguages}</h3><SettingsHelp text={guidance.catalogHelp} label={I18N.settings.helpLabel} />{requiresStop && <SettingsHelp text={I18N.settings.languageChangeRequiresStop} label={I18N.settings.helpLabel} icon="lock" />}</header>
    {!hideSourceLanguage && <SettingsRow label={I18N.settings.sourceLanguage} description={guidance.help} feedback={(sourceNotice || (appleSpeech && onOpenAppleResources)) && <>
      {sourceNotice && <span className="recognition-language-notice">{sourceNotice}</span>}
      {appleSpeech && onOpenAppleResources && <button type="button" className="settings-link" onClick={onOpenAppleResources}>{I18N.settings.appleSpeechOpenResources}</button>}
    </>} align="start">
      <LanguageChoices label={I18N.settings.sourceLanguage} value={settings.sourceLanguage} valueLabel={appleSpeech ? guidance.optionLabel(settings.sourceLanguage) : undefined}
        disabled={disabled || busy || sources.length === 0 || (sources.length === 1 && sources[0] === settings.sourceLanguage)}
        options={sources.map(value => ({ value, label: guidance.optionLabel(value) }))}
        onChange={value => { const sourceLanguage = sources.find(language => language === value); if (sourceLanguage) void save({ sourceLanguage }); }} />
    </SettingsRow>}
    {targets.includes("original") && <SettingsRow label={I18N.settings.skipTranslation} description={I18N.settings.skipTranslationHelp}>
      <Switch aria-label={I18N.settings.skipTranslation} checked={skipped} disabled={disabled || busy}
        onChange={skip => {
          const previous = previousTarget.current;
          const targetLanguage = skip ? "original" : previous !== "original" && targets.includes(previous) ? previous : targets.find(target => target !== "original");
          if (targetLanguage) void save({ targetLanguage });
        }} />
    </SettingsRow>}
    <SettingsRow label={I18N.settings.translateTo} description={I18N.settings.translationConfiguredHelp} align="start">
      <LanguageChoices label={I18N.settings.translateTo} value={settings.targetLanguage} disabled={disabled || busy || skipped || targets.length === 1}
        options={targets.map(value => ({ value, label: TARGET_LANGUAGE_DISPLAY_NAMES[value] }))}
        onChange={value => { const targetLanguage = targets.find(language => language === value); if (targetLanguage) void save({ targetLanguage }); }} />
    </SettingsRow>
  </section>;
}

function LanguageChoices({ label, value, valueLabel, options, disabled, onChange }: {
  label: string; value: string; valueLabel?: string; options: readonly { value: string; label: string }[]; disabled: boolean; onChange: (value: string) => void;
}) {
  if (options.length > 6 || (valueLabel !== undefined && !options.some(option => option.value === value))) return <LanguageSelect label={label} value={value} valueLabel={valueLabel} options={options} disabled={disabled} onChange={onChange} />;
  return <div className="profile-language-choices" role="group" aria-label={label}>
    {options.map(option => <button key={option.value} type="button" className={`profile-language-choice${option.value === value ? " is-selected" : ""}`} aria-pressed={option.value === value}
      disabled={disabled} onClick={() => { if (option.value !== value) onChange(option.value); }}>
      <span>{option.label}</span>{option.value === value && <Icon name="checkmark" />}
    </button>)}
  </div>;
}
