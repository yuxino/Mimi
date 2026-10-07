import { useLanguageNormalizationToast } from "./useLanguageNormalizationToast";
import { useEffect, useRef, useState } from "react";
import { Switch } from "../../components/Switch";
import { LanguageSelect } from "../../components/LanguageSelect";
import { speechLanguageGuidance, targetLanguageOptionLabel } from "../../lib/speechLanguageGuidance";
import { I18N } from "../../lib/i18n";
import { languageActionErrorMessage } from "../../lib/connectionDiagnostics";
import { activeServiceProfile, sourceLanguagesForSettings, targetLanguagesForSettings, textTranslationForProfile } from "../../lib/providerCapabilities";
import { useStore } from "../../lib/store";
import { type SettingsDraft, type SettingsSnapshot } from "../../lib/types";
import { SettingsHelp } from "./SettingsHelp";
import { SettingsRow } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

/** Explicit language preferences belong to the active service. */
export function ProfileLanguageSettings({ settings, disabled, onOpenAppleResources, hideSourceLanguage = false, embedded = false, onBusyChange }: { settings: SettingsSnapshot; disabled: boolean; onOpenAppleResources?: () => void; hideSourceLanguage?: boolean; embedded?: boolean; onBusyChange?: (busy: boolean) => void }) {
  const saveSettings = useStore(state => state.saveSettings);
  const switchSourceLanguage = useStore(state => state.switchSourceLanguage);
  const switchTargetLanguage = useStore(state => state.switchTargetLanguage);
  const session = useStore(state => state.session);
  const [busy, setBusy] = useState(false);
  const { beginToast } = useSettingsToast();
  const trackLanguageChange = useLanguageNormalizationToast(settings);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const busyCallback = useRef(onBusyChange);
  useEffect(() => { busyCallback.current = onBusyChange; }, [onBusyChange]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; if (inFlight.current) busyCallback.current?.(false); }; }, []);
  const guidance = speechLanguageGuidance(settings);
  const sources = sourceLanguagesForSettings(settings);
  const appleSpeech = activeServiceProfile(settings)?.provider === "appleSpeech";
  const activeProfile = activeServiceProfile(settings);
  const appleTranslation = activeProfile && textTranslationForProfile(activeProfile) === "apple";
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
    busyCallback.current?.(true);
    const notify = beginToast();
    const finishLanguageChange = trackLanguageChange(draft.targetLanguage, notify);
    try {
      if (session.isActive || session.isPaused) {
        if (draft.sourceLanguage !== undefined) await switchSourceLanguage(draft.sourceLanguage);
        else if (draft.targetLanguage !== undefined) await switchTargetLanguage(draft.targetLanguage);
      } else {
        await saveSettings(draft);
      }
      if (mounted.current) notify(I18N.settings.languageSaved);
      finishLanguageChange(true);
    } catch (error) {
      finishLanguageChange(false);
      if (mounted.current) notify(languageActionErrorMessage(error, I18N.settings.languageSaveFailed), true);
    } finally {
      inFlight.current = false;
      if (mounted.current) { setBusy(false); busyCallback.current?.(false); }
    }
  };
  if (!embedded && hideSourceLanguage && skipped && targets.every(target => target === "original")) return null;
  const controls = <>
    {!hideSourceLanguage && <SettingsRow label={I18N.settings.sourceLanguage} description={guidance.help} feedback={(sourceNotice || (appleSpeech && onOpenAppleResources)) && <>
      {sourceNotice && <span className="recognition-language-notice">{sourceNotice}</span>}
      {appleSpeech && onOpenAppleResources && <button type="button" className="settings-link" onClick={onOpenAppleResources}>{I18N.settings.appleSpeechOpenResources}</button>}
    </>}>
      <LanguageSelect label={I18N.settings.sourceLanguage} value={settings.sourceLanguage} valueLabel={appleSpeech || appleTranslation ? guidance.optionLabel(settings.sourceLanguage) : undefined}
        disabled={disabled || busy || sources.length === 0 || (sources.length === 1 && sources[0] === settings.sourceLanguage)}
        options={sources.map(value => ({ value, label: guidance.optionLabel(value) }))}
        onChange={value => { const sourceLanguage = sources.find(language => language === value); if (sourceLanguage) void save({ sourceLanguage }); }} />
    </SettingsRow>}
    {!embedded && targets.includes("original") && <SettingsRow label={I18N.settings.skipTranslation} description={I18N.settings.skipTranslationHelp}>
      <Switch aria-label={I18N.settings.skipTranslation} checked={skipped} disabled={disabled || busy || (skipped && targets.every(target => target === "original"))}
        onChange={skip => {
          const previous = previousTarget.current;
          const targetLanguage = skip ? "original" : previous !== "original" && targets.includes(previous) ? previous : targets.find(target => target !== "original");
          if (targetLanguage) void save({ targetLanguage });
        }} />
    </SettingsRow>}
    <SettingsRow label={I18N.settings.translateTo} description={I18N.settings.translationConfiguredHelp}>
      <LanguageSelect label={I18N.settings.translateTo} value={settings.targetLanguage} valueLabel={appleTranslation ? targetLanguageOptionLabel(settings, settings.targetLanguage) : undefined} disabled={disabled || busy || (!embedded && skipped) || (targets.length === 1 && targets[0] === settings.targetLanguage)}
        options={targets.map(value => ({ value, label: targetLanguageOptionLabel(settings, value) }))}
        onChange={value => { const targetLanguage = targets.find(language => language === value); if (targetLanguage) void save({ targetLanguage }); }} />
    </SettingsRow>
  </>;
  if (embedded) return <div className="apple-translation-language-controls" aria-busy={busy}>{controls}</div>;
  return <section id="translation-languages" className="profile-language-settings" aria-labelledby="translation-languages-title" aria-busy={busy}>
    <header className="profile-language-settings__heading"><h3 id="translation-languages-title">{I18N.settings.subtitleLanguages}</h3><SettingsHelp text={guidance.catalogHelp} label={I18N.settings.helpLabel} /></header>
    {controls}
  </section>;
}
