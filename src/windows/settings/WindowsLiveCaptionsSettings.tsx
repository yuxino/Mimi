import { useEffect, useRef, useState, type ComponentProps } from "react";
import { RotateCw } from "lucide-react";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { openWindowsLiveCaptions } from "../../lib/ipc";
import { textTranslationForProfile } from "../../lib/providerCapabilities";
import { useStore } from "../../lib/store";
import { windowsLiveCaptionsStatusText } from "../../lib/windowsLiveCaptions";
import type { WindowsLiveCaptionsSupport } from "../../lib/types";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";
import { InlineFeedback } from "./SettingsPrimitives";
import { SettingsHelp } from "./SettingsHelp";

type Props = ComponentProps<typeof AlibabaCredentialEditor> & {
  windowsSupport: WindowsLiveCaptionsSupport | null;
  loading: boolean;
  failed: boolean;
  consentControlsDisabled?: boolean;
  onRetry: () => Promise<void>;
  onBusyChange: (busy: boolean) => void;
};

export function WindowsLiveCaptionsSettings({ windowsSupport, loading, failed, consentControlsDisabled = false, onRetry, onBusyChange, requiresStop = false, ...editor }: Props) {
  const [helpOpen, setHelpOpen] = useState(false);
  const [allowsReading, setAllowsReading] = useState(false);
  const [microphoneOff, setMicrophoneOff] = useState(false);
  const [operation, setOperation] = useState<"open" | "refresh" | "consent" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const stop = useStore(state => state.stop);
  const setConsent = useStore(state => state.setWindowsLiveCaptionsConsent);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const consented = editor.profile.windowsLiveCaptionsConsent === true;
  const supported = windowsSupport?.available === true && windowsSupport.status !== "unsupported";
  const controlDisabled = consentControlsDisabled || operation !== null || editor.busy || editor.readOnly;
  const guideId = `${editor.inputId}-windows-guide`;
  const privacyId = `${editor.inputId}-windows-privacy`;
  const run = async (action: NonNullable<typeof operation>, work: () => Promise<unknown>, failure: string) => {
    if (inFlight.current || controlDisabled) return;
    inFlight.current = true;
    setOperation(action);
    setError(null);
    onBusyChange(true);
    try { await work(); }
    catch { if (mounted.current) setError(failure); }
    finally {
      inFlight.current = false;
      if (mounted.current) { setOperation(null); onBusyChange(false); }
    }
  };
  const changeConsent = () => {
    if (!consented && (!supported || loading || requiresStop || !allowsReading || !microphoneOff)) return;
    void run("consent", async () => {
      // Revocation remains accessible during a session; failure to stop must
      // never change the permission or leave an active reader unauthorized.
      if (requiresStop) await stop();
      await setConsent(editor.profile.id, !consented);
      if (mounted.current) { setAllowsReading(false); setMicrophoneOff(false); }
    }, I18N.settings.windowsLiveCaptionsConsentFailed);
  };
  const route = textTranslationForProfile(editor.profile);
  const originalOnly = editor.targetLanguage === "original";
  return <div className="credential-panel windows-live-captions-settings">
    <section className="service-stage" aria-labelledby={`${editor.inputId}-windows-title`} aria-busy={loading || operation !== null}>
      <header className="service-stage__heading">
        <div className="service-stage__name-help"><h3 id={`${editor.inputId}-windows-title`}>{I18N.settings.speechRecognition}</h3>
          <SettingsHelp text={I18N.settings.windowsLiveCaptionsDescription} label={I18N.settings.helpLabel} /></div>
        <button type="button" className="settings-link apple-speech-tutorial-toggle" aria-expanded={helpOpen} aria-controls={guideId} onClick={() => setHelpOpen(open => !open)}>
          {I18N.settings.windowsLiveCaptionsSetup}<Icon name={helpOpen ? "chevron-up" : "chevron-down"} />
        </button>
      </header>
      <p className="windows-live-captions-note">{I18N.settings.windowsLiveCaptionsDescription}</p>
      {helpOpen && <div id={guideId} className="apple-speech-tutorial" role="region" aria-label={I18N.settings.windowsLiveCaptionsSetup}>
        <ol><li>{I18N.settings.windowsLiveCaptionsStepOpen}</li><li>{I18N.settings.windowsLiveCaptionsStepLanguage}</li><li>{I18N.settings.windowsLiveCaptionsStepMicrophone}</li></ol>
      </div>}
      <InlineFeedback tone={failed || windowsSupport?.status === "unreadable" ? "error" : "info"}>
        <span role="status">{loading ? I18N.settings.windowsLiveCaptionsLoading : failed ? I18N.settings.windowsLiveCaptionsLoadFailed : windowsLiveCaptionsStatusText(windowsSupport)}</span>
      </InlineFeedback>
      <div className="windows-live-captions-actions">
        {supported && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={controlDisabled || loading}
          onClick={() => void run("open", async () => { await openWindowsLiveCaptions(); await onRetry(); }, I18N.settings.windowsLiveCaptionsOpenFailed)}>
          <Icon name="app-window" />{I18N.settings.windowsLiveCaptionsOpen}
        </button>}
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={controlDisabled || loading}
          onClick={() => void run("refresh", onRetry, I18N.settings.windowsLiveCaptionsLoadFailed)}>
          <RotateCw size={14} aria-hidden="true" />{I18N.settings.windowsLiveCaptionsRefresh}
        </button>
      </div>
      <p id={privacyId} className="windows-live-captions-note">{originalOnly ? I18N.settings.windowsLiveCaptionsOriginalPrivacy : I18N.settings.windowsLiveCaptionsPrivacy}</p>
      {consented ? <div className="windows-live-captions-actions">
        <span className="apple-speech-resource-status"><Icon name="checkmark-circle" />{I18N.settings.windowsLiveCaptionsAllowed}</span>
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={controlDisabled} onClick={changeConsent}>
          {requiresStop ? I18N.settings.windowsLiveCaptionsStopAndRevoke : I18N.settings.windowsLiveCaptionsRevoke}
        </button>
      </div> : <fieldset className="windows-live-captions-consent" disabled={controlDisabled || !supported || loading || requiresStop} aria-describedby={privacyId}>
        <legend className="settings-sr-only">{I18N.settings.windowsLiveCaptionsAllow}</legend>
        <label><input type="checkbox" checked={allowsReading} onChange={event => setAllowsReading(event.currentTarget.checked)} /><span>{I18N.settings.windowsLiveCaptionsConsent}</span></label>
        <label><input type="checkbox" checked={microphoneOff} onChange={event => setMicrophoneOff(event.currentTarget.checked)} /><span>{I18N.settings.windowsLiveCaptionsMicrophoneOff}</span></label>
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={!allowsReading || !microphoneOff} onClick={changeConsent}>{I18N.settings.windowsLiveCaptionsAllow}</button>
      </fieldset>}
      {requiresStop && !consented && <InlineFeedback tone="info">{I18N.settings.profileCreateRequiresStop}</InlineFeedback>}
      {error && <InlineFeedback tone="error">{error}</InlineFeedback>}
      <p className="windows-live-captions-note">{I18N.settings.windowsLiveCaptionsAudioUnavailable}</p>
      {supported && consented && windowsSupport?.status === "ready" && <div className="apple-speech-connection-check">{typeof editor.connectionCheck === "function" ? editor.connectionCheck(undefined) : editor.connectionCheck}</div>}
    </section>
    {!originalOnly && route === "followService" && <InlineFeedback tone="info">{I18N.settings.windowsLiveCaptionsNoTranslator}</InlineFeedback>}
    <AlibabaCredentialEditor {...editor} textOnly requiresStop={requiresStop} connectionCheck={undefined} disabled={editor.disabled || operation !== null} />
  </div>;
}
