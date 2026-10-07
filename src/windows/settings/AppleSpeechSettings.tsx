import { useEffect, useMemo, useRef, useState, type ComponentProps } from "react";
import { LanguageSelect } from "../../components/LanguageSelect";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { languageActionErrorMessage } from "../../lib/connectionDiagnostics";
import { activeServiceProfile, sourceLanguagesForSettings, textTranslationForProfile, textTranslationSourceLanguages } from "../../lib/providerCapabilities";
import { useStore } from "../../lib/store";
import { getAppleTranslationStatus, getAppleTranslationSupport, prepareAppleSpeechLanguage } from "../../lib/ipc";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, type AppleSpeechSupport, type SettingsSnapshot, type SourceLanguage } from "../../lib/types";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";
import { InlineFeedback, SettingsRow } from "./SettingsPrimitives";
import { SettingsHelp } from "./SettingsHelp";
import { useSettingsToast } from "./useSettingsToast";

type Props = ComponentProps<typeof AlibabaCredentialEditor> & {
  support: AppleSpeechSupport | null;
  settings: SettingsSnapshot;
  requiresStop?: boolean;
  loading: boolean;
  failed: boolean;
  sourceLanguage: SourceLanguage;
  onRetry: () => Promise<void>;
  onPrepared: (support: AppleSpeechSupport) => void;
  onBusyChange: (busy: boolean) => void;
  onSelectProfile?: (profileId: string, sourceLanguage: SourceLanguage) => Promise<void>;
};

export function AppleSpeechSettings({ support, settings, requiresStop = false, loading, failed, sourceLanguage, onRetry, onPrepared, onBusyChange, onSelectProfile, ...editor }: Props) {
  const [selected, setSelected] = useState<{ language: string; context: symbol } | null>(null);
  const selectionKey = `${settings.activeProfileId}:${settings.sourceLanguage}`;
  const selectionContext = useMemo(() => Symbol(selectionKey), [selectionKey]);
  const [helpOpen, setHelpOpen] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [savingLanguage, setSavingLanguage] = useState(false);
  const currentSettings = useRef(settings);
  useEffect(() => { currentSettings.current = settings; }, [settings]);
  const saveSettings = useStore(state => state.saveSettings);
  const [prepareFailed, setPrepareFailed] = useState(false);
  const [languageError, setLanguageError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const { beginToast } = useSettingsToast();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const languages = support?.languages ?? [];
  const installedLanguages = languages.filter(item => item.installed);
  const active = settings.activeProfileId === editor.profile.id && editor.profile.provider === "appleSpeech";
  const route = textTranslationForProfile(editor.profile);
  const routeSources = active ? sourceLanguagesForSettings(settings) : installedLanguages.filter(item =>
    settings.targetLanguage === "original" || (route !== "deepL" && route !== "deepLX")
    || textTranslationSourceLanguages(route).includes(item.sourceLanguage)).map(item => item.sourceLanguage);
  const recognitionLanguages = installedLanguages.filter(item => routeSources.includes(item.sourceLanguage));
  const currentSource = active ? settings.sourceLanguage : sourceLanguage;
  const language = languages.find(item => item.sourceLanguage === (selected?.context === selectionContext ? selected.language : currentSource)) ?? recognitionLanguages[0] ?? languages[0];
  const routeAllowsLanguage = !!language && routeSources.includes(language.sourceLanguage);
  const languageInUse = active && routeAllowsLanguage && language?.sourceLanguage === currentSource;
  const textAllowsLanguage = !!language && (settings.targetLanguage === "original" || (route !== "deepL" && route !== "deepLX")
    || textTranslationSourceLanguages(route).includes(language.sourceLanguage));
  const canUseLanguage = support?.available && language && textAllowsLanguage
    && (!language.installed || routeAllowsLanguage) && (!languageInUse || !language.installed) && (active || !!onSelectProfile);
  const busy = preparing || savingLanguage;
  const disabled = editor.disabled || editor.busy || busy || requiresStop;
  const applyLanguage = async () => {
    if (!language || !canUseLanguage || disabled || inFlight.current) return;
    const chosen = language;
    const activeProfileId = settings.activeProfileId;
    let downloaded = chosen.installed;
    let stage: "translation-check" | "download" | "save" = downloaded ? "save" : "download";
    inFlight.current = true;
    setPrepareFailed(false);
    setLanguageError(null);
    setPreparing(!downloaded);
    setSavingLanguage(downloaded);
    onBusyChange(true);
    const notify = beginToast();
    try {
      // The recognition inventory includes downloadable languages that Apple's
      // separate translator may not support. Check before requesting assets.
      if (route === "apple" && settings.targetLanguage !== "original") {
        stage = "translation-check";
        if (chosen.sourceLanguage === settings.targetLanguage && chosen.sourceLanguage !== "zh_en") {
          const textSupport = await getAppleTranslationSupport();
          if (!textSupport.available || !textSupport.sourceLanguages.includes(chosen.sourceLanguage)
            || !textSupport.targetLanguages.includes(settings.targetLanguage)) throw new Error("apple_translation_unavailable");
        } else {
          const status = await getAppleTranslationStatus(chosen.sourceLanguage, settings.targetLanguage);
          if (status === "unavailable") throw new Error("apple_translation_unavailable");
          if (status === "unsupported") throw new Error("apple_translation_language_unsupported");
          if (status !== "installed" && status !== "supported") throw new Error("apple_translation_status_failed");
        }
        if (!mounted.current) return;
        const current = currentSettings.current;
        if (current.activeProfileId !== activeProfileId) throw new Error("source_switch_profile");
        const currentProfile = current.profiles.find(profile => profile.id === editor.profile.id);
        if (current.targetLanguage !== settings.targetLanguage || !currentProfile
          || textTranslationForProfile(currentProfile) !== route) throw new Error("language_switch_superseded");
      }
      stage = downloaded ? "save" : "download";
      if (!downloaded) {
        const result = await prepareAppleSpeechLanguage(chosen.sourceLanguage);
        if (!mounted.current) return;
        onPrepared(result);
        if (!result.languages.some(item => item.sourceLanguage === chosen.sourceLanguage && item.installed)) throw new Error("resources-not-installed");
        downloaded = true;
        stage = "save";
        setPreparing(false);
        setSavingLanguage(true);
      }
      const current = currentSettings.current;
      if (current.activeProfileId !== activeProfileId) throw new Error("source_switch_profile");
      if (!active && onSelectProfile) {
        await onSelectProfile(editor.profile.id, chosen.sourceLanguage);
      } else {
        const profile = activeServiceProfile(current);
        if (profile?.id !== editor.profile.id || profile.provider !== "appleSpeech") throw new Error("source_switch_profile");
        // Preparation refreshes native capabilities before returning. The save
        // validates that fresh route even if its broadcast has not rendered yet.
        await saveSettings({ sourceLanguage: chosen.sourceLanguage });
      }
      if (mounted.current) { setSelected(null); notify(I18N.settings.appleSpeechLanguageSelected); }
    } catch (error) {
      if (mounted.current) {
        const message = stage === "translation-check" ? languageActionErrorMessage(error, I18N.settings.appleTranslationStatusFailed)
          : downloaded ? languageActionErrorMessage(error, I18N.settings.languageSaveFailed) : I18N.settings.appleSpeechPrepareFailed;
        setPrepareFailed(stage === "download");
        setLanguageError(message);
        notify(message, true);
      }
    } finally {
      inFlight.current = false;
      if (mounted.current) { setPreparing(false); setSavingLanguage(false); onBusyChange(false); }
    }
  };
  const tutorialId = `${editor.inputId}-apple-tutorial`;
  return <div className="credential-panel apple-speech-settings">
    <section id="apple-speech-resources" tabIndex={-1} className="service-stage" aria-labelledby={`${editor.inputId}-apple-title`} aria-busy={loading || busy}>
      <header className="service-stage__heading">
        <div className="service-stage__name-help"><h3 id={`${editor.inputId}-apple-title`}>{I18N.settings.speechRecognition}</h3><SettingsHelp text={I18N.settings.appleSpeechDescription} label={I18N.settings.helpLabel} /></div>
        <button type="button" className="settings-link apple-speech-tutorial-toggle" aria-expanded={helpOpen} aria-controls={tutorialId} onClick={() => setHelpOpen(open => !open)}>
          {I18N.settings.appleSpeechInstallHelp}<Icon name={helpOpen ? "chevron-up" : "chevron-down"} />
        </button>
      </header>
      {helpOpen && <div id={tutorialId} className="apple-speech-tutorial" role="region" aria-label={I18N.settings.appleSpeechInstallHelp}>
        <p>{I18N.settings.appleSpeechDownloadLocation}</p>
        <ol><li>{I18N.settings.appleSpeechInstallStepChoose}</li><li>{I18N.settings.appleSpeechInstallStepDownload}</li><li>{I18N.settings.appleSpeechInstallStepUse}</li></ol>
      </div>}
      {loading ? <InlineFeedback tone="info">{I18N.settings.appleSpeechLoading}</InlineFeedback>
        : failed ? <InlineFeedback tone="error">{I18N.settings.appleSpeechLoadFailed} <button type="button" className="settings-link" disabled={disabled} onClick={() => void onRetry()}>{I18N.settings.retryLoadingSettings}</button></InlineFeedback>
        : !support?.available || !language ? <InlineFeedback tone="info">{I18N.settings.appleSpeechUnavailable}</InlineFeedback>
        : <>
          <SettingsRow label={I18N.settings.sourceLanguage} description={I18N.settings.appleSpeechResourcesHelp}>
            <LanguageSelect label={I18N.settings.sourceLanguage} value={language.sourceLanguage}
              disabled={disabled}
              options={languages.map(item => ({ value: item.sourceLanguage, label: `${SOURCE_LANGUAGE_DISPLAY_NAMES[item.sourceLanguage]}${item.installed ? "" : ` · ${I18N.settings.appleSpeechNotInstalled}`}` }))}
              onChange={value => { setSelected({ language: value, context: selectionContext }); setPrepareFailed(false); setLanguageError(null); }} />
          </SettingsRow>
          <div className="apple-speech-resource-actions">
            <span className="apple-speech-resource-status" role="status">
              {preparing ? <span className="settings-spinner" aria-hidden="true" /> : <Icon name={language.installed ? "checkmark-circle" : "download"} />}
              {language.locale} · {preparing ? I18N.settings.appleSpeechPreparing : languageInUse && language.installed ? I18N.settings.appleSpeechLanguageInUse
                : language.installed ? I18N.settings.appleSpeechInstalled : I18N.settings.appleSpeechNotInstalled}
            </span>
            {canUseLanguage && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={() => void applyLanguage()}>
              <Icon name={language.installed ? "checkmark" : "download"} />
              {language.installed ? I18N.settings.appleSpeechUseLanguage : prepareFailed ? I18N.settings.appleSpeechRetryDownload : I18N.settings.appleSpeechDownloadAndUse}
            </button>}
          </div>
          {requiresStop && <span className="apple-speech-resource-note">{I18N.settings.languageChangeRequiresStop}</span>}
          {!requiresStop && (!textAllowsLanguage || (language.installed && !routeAllowsLanguage)) && <InlineFeedback tone="info">{I18N.settings.appleSpeechLanguageRouteUnsupported}</InlineFeedback>}
          {languageError && <InlineFeedback tone="error">{languageError}</InlineFeedback>}
          <div className="apple-speech-connection-check">{typeof editor.connectionCheck === "function" ? editor.connectionCheck(undefined, language.sourceLanguage) : editor.connectionCheck}</div>
        </>}
    </section>
    <AlibabaCredentialEditor {...editor} textOnly connectionCheck={undefined} disabled={disabled} sourceLanguage={settings.sourceLanguage} targetLanguage={settings.targetLanguage} requiresStop={requiresStop} />
  </div>;
}
