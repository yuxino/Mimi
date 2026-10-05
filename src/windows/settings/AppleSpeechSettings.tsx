import { useEffect, useRef, useState, type ComponentProps } from "react";
import { LanguageSelect } from "../../components/LanguageSelect";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { languageActionErrorMessage } from "../../lib/connectionDiagnostics";
import { activeServiceProfile, sourceLanguagesForSettings, textTranslationForProfile } from "../../lib/providerCapabilities";
import { useStore } from "../../lib/store";
import { prepareAppleSpeechLanguage } from "../../lib/ipc";
import { LEGACY_SOURCE_LANGUAGE_CASES, SOURCE_LANGUAGE_DISPLAY_NAMES, type AppleSpeechSupport, type SettingsSnapshot, type SourceLanguage } from "../../lib/types";
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
  const [selected, setSelected] = useState<string | null>(null);
  const [packSelection, setPackSelection] = useState<string | null>(null);
  const [managerOpen, setManagerOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [savingLanguage, setSavingLanguage] = useState(false);
  const currentSettings = useRef(settings);
  useEffect(() => { currentSettings.current = settings; }, [settings]);
  const saveSettings = useStore(state => state.saveSettings);
  const [prepareFailed, setPrepareFailed] = useState(false);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const { beginToast } = useSettingsToast();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const languages = support?.languages ?? [];
  const installedLanguages = languages.filter(item => item.installed);
  const active = settings.activeProfileId === editor.profile.id && editor.profile.provider === "appleSpeech";
  const route = textTranslationForProfile(editor.profile);
  const routeSources = active ? sourceLanguagesForSettings(settings) : installedLanguages.filter(item =>
    settings.targetLanguage === "original" || !["deepL", "deepLX"].includes(route)
    || LEGACY_SOURCE_LANGUAGE_CASES.includes(item.sourceLanguage)).map(item => item.sourceLanguage);
  const recognitionLanguages = installedLanguages.filter(item => routeSources.includes(item.sourceLanguage));
  const currentSource = active ? settings.sourceLanguage : sourceLanguage;
  const language = languages.find(item => item.sourceLanguage === (selected ?? currentSource)) ?? recognitionLanguages[0] ?? languages[0];
  const pack = languages.find(item => item.sourceLanguage === packSelection) ?? languages.find(item => !item.installed) ?? language;
  const routeAllowsLanguage = !!language && routeSources.includes(language.sourceLanguage);
  const languageInUse = active && routeAllowsLanguage && language?.sourceLanguage === currentSource;
  const canUseLanguage = support?.available && language?.installed && routeAllowsLanguage && (active ? !languageInUse : !!onSelectProfile);
  const busy = preparing || savingLanguage;
  const disabled = editor.disabled || editor.busy || busy || requiresStop;
  const prepare = async () => {
    if (!pack || disabled || inFlight.current || pack.installed) return;
    inFlight.current = true;
    setPreparing(true);
    setPrepareFailed(false);
    onBusyChange(true);
    const notify = beginToast();
    try {
      const result = await prepareAppleSpeechLanguage(pack.sourceLanguage);
      if (!mounted.current) return;
      onPrepared(result);
      if (!result.languages.some(item => item.sourceLanguage === pack.sourceLanguage && item.installed)) throw new Error("resources-not-installed");
      setSelected(pack.sourceLanguage);
      setManagerOpen(false);
      notify(I18N.settings.appleSpeechPrepared);
    } catch {
      if (mounted.current) {
        setPrepareFailed(true);
        notify(I18N.settings.appleSpeechPrepareFailed, true);
      }
    } finally {
      inFlight.current = false;
      if (mounted.current) { setPreparing(false); onBusyChange(false); }
    }
  };
  const applyLanguage = async () => {
    if (!language || !canUseLanguage || disabled || inFlight.current) return;
    inFlight.current = true;
    setSavingLanguage(true);
    onBusyChange(true);
    const notify = beginToast();
    try {
      const current = currentSettings.current;
      const profile = activeServiceProfile(current);
      if (!active && onSelectProfile) {
        // Selecting the language is an explicit request to use this profile.
        // The parent owns the profile switch and its resulting language route.
        await onSelectProfile(editor.profile.id, language.sourceLanguage);
      } else {
        if (profile?.id !== editor.profile.id || profile.provider !== "appleSpeech") throw new Error("source_switch_profile");
        if (!sourceLanguagesForSettings(current).includes(language.sourceLanguage)) throw new Error("source_switch_unsupported");
        await saveSettings({ sourceLanguage: language.sourceLanguage });
      }
      if (mounted.current) notify(I18N.settings.appleSpeechLanguageSelected);
    } catch (error) {
      if (mounted.current) notify(languageActionErrorMessage(error, I18N.settings.languageSaveFailed), true);
    } finally {
      inFlight.current = false;
      if (mounted.current) { setSavingLanguage(false); onBusyChange(false); }
    }
  };
  const tutorialId = `${editor.inputId}-apple-tutorial`;
  const managerId = `${editor.inputId}-apple-packs`;
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
          <SettingsRow label={I18N.settings.sourceLanguage} description={I18N.settings.appleSpeechResourcesHelp}
            feedback={requiresStop ? <InlineFeedback tone="info">{I18N.settings.languageChangeRequiresStop}</InlineFeedback>
              : language.installed && !routeAllowsLanguage ? <InlineFeedback tone="info">{I18N.settings.appleSpeechLanguageRouteUnsupported}</InlineFeedback> : undefined}>
            <LanguageSelect label={I18N.settings.sourceLanguage} value={language.sourceLanguage}
              valueLabel={SOURCE_LANGUAGE_DISPLAY_NAMES[language.sourceLanguage]} disabled={disabled || recognitionLanguages.length === 0}
              options={recognitionLanguages.map(item => ({ value: item.sourceLanguage, label: SOURCE_LANGUAGE_DISPLAY_NAMES[item.sourceLanguage] }))}
              onChange={value => { setSelected(value); setPrepareFailed(false); }} />
          </SettingsRow>
          <div className="apple-speech-resource-actions">
            <span className="apple-speech-resource-status" role="status">
              <Icon name={language.installed ? "checkmark-circle" : "download"} />
              {language.locale} · {languageInUse && language.installed ? I18N.settings.appleSpeechLanguageInUse
                : language.installed ? I18N.settings.appleSpeechInstalled : I18N.settings.appleSpeechNotInstalled}
            </span>
            {canUseLanguage && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={() => void applyLanguage()}><Icon name="checkmark" />{I18N.settings.appleSpeechUseLanguage}</button>}
            <button type="button" className="settings-button settings-button--quiet settings-button--compact" aria-expanded={managerOpen} aria-controls={managerId} disabled={busy} onClick={() => { setManagerOpen(open => !open); setPrepareFailed(false); }}><Icon name="plus" />{I18N.settings.appleSpeechAddLanguagePack}</button>
          </div>
          {managerOpen && pack && <div id={managerId} className="apple-speech-pack-manager" role="region" aria-label={I18N.settings.appleSpeechAddLanguagePack}>
            <SettingsRow label={I18N.settings.appleSpeechLanguagePack}
              feedback={prepareFailed && <InlineFeedback tone="error">{I18N.settings.appleSpeechPrepareFailed}</InlineFeedback>}>
              <LanguageSelect label={I18N.settings.appleSpeechLanguagePack} value={pack.sourceLanguage} disabled={disabled}
                options={languages.map(item => ({ value: item.sourceLanguage, label: `${SOURCE_LANGUAGE_DISPLAY_NAMES[item.sourceLanguage]} · ${item.installed ? I18N.settings.appleSpeechInstalled : I18N.settings.appleSpeechNotInstalled}` }))}
                onChange={value => { setPackSelection(value); setPrepareFailed(false); }} />
            </SettingsRow>
            <div className="apple-speech-resource-actions">
              <span className="apple-speech-resource-status" role="status">{preparing && <span className="settings-spinner" aria-hidden="true" />}{pack.locale} · {preparing ? I18N.settings.appleSpeechPreparing : pack.installed ? I18N.settings.appleSpeechInstalled : I18N.settings.appleSpeechNotInstalled}</span>
              {!pack.installed && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={() => void prepare()}><Icon name="download" />{prepareFailed ? I18N.settings.appleSpeechRetryDownload : I18N.settings.appleSpeechPrepare}</button>}
            </div>
          </div>}
          <div className="apple-speech-connection-check">{typeof editor.connectionCheck === "function" ? editor.connectionCheck(undefined, language.sourceLanguage) : editor.connectionCheck}</div>
        </>}
    </section>
    <AlibabaCredentialEditor {...editor} textOnly connectionCheck={undefined} disabled={disabled} />
  </div>;
}
