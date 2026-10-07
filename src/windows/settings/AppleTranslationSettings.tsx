import { useEffect, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { credentialErrorMessage } from "../../lib/connectionDiagnostics";
import { prepareAppleTranslationLanguages } from "../../lib/ipc";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type AppleTranslationSupport, type SourceLanguage, type TargetLanguage } from "../../lib/types";
import { InlineFeedback } from "./SettingsPrimitives";
import { useAppleTranslationStatus } from "./useAppleTranslationSupport";
import { useSettingsToast } from "./useSettingsToast";

export function AppleTranslationSettings({ profileId, sourceLanguage, targetLanguage, support, loading, failed, visible = true, disabled, requiresStop = false, onRetry, onBusyChange, onPrepared }: {
  profileId: string;
  sourceLanguage: SourceLanguage;
  targetLanguage: TargetLanguage;
  support: AppleTranslationSupport | null;
  loading: boolean;
  failed: boolean;
  visible?: boolean;
  disabled: boolean;
  requiresStop?: boolean;
  onRetry: () => Promise<void>;
  onBusyChange?: (busy: boolean) => void;
  onPrepared?: () => void;
}) {
  const [preparing, setPreparing] = useState(false);
  const [prepareError, setPrepareError] = useState<{ key: string; message: string } | null>(null);
  const sameLanguage = sourceLanguage === targetLanguage;
  const supportedPair = sourceLanguage !== "auto" && targetLanguage !== "original" && !sameLanguage
    && support?.sourceLanguages.includes(sourceLanguage) && support?.targetLanguages.includes(targetLanguage);
  const pair = useAppleTranslationStatus(profileId, sourceLanguage, targetLanguage, visible && !!support?.available && !!supportedPair);
  const key = JSON.stringify([profileId, sourceLanguage, targetLanguage, visible]);
  const currentKey = useRef(key);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const callbacks = useRef({ onBusyChange, onPrepared });
  useEffect(() => { callbacks.current = { onBusyChange, onPrepared }; }, [onBusyChange, onPrepared]);
  useEffect(() => { currentKey.current = key; }, [key]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (inFlight.current) callbacks.current.onBusyChange?.(false);
    };
  }, []);
  const { beginToast } = useSettingsToast();
  const prepare = async () => {
    if (sourceLanguage === "auto" || targetLanguage === "original" || !supportedPair || pair.status !== "supported" || disabled || requiresStop || inFlight.current) return;
    const requestKey = key;
    inFlight.current = true;
    setPreparing(true);
    setPrepareError(null);
    callbacks.current.onBusyChange?.(true);
    const notify = beginToast();
    try {
      const status = await prepareAppleTranslationLanguages(sourceLanguage, targetLanguage);
      if (!mounted.current || currentKey.current !== requestKey) return;
      pair.update(status);
      callbacks.current.onPrepared?.();
      if (status === "installed") notify(I18N.settings.appleTranslationPrepared);
      else {
        const message = status === "unsupported" ? I18N.settings.appleTranslationUnsupported : status === "unavailable" ? I18N.settings.appleTranslationUnavailable : I18N.settings.appleTranslationPrepareFailed;
        setPrepareError({ key: requestKey, message });
        notify(message, true);
      }
    } catch (error) {
      if (mounted.current && currentKey.current === requestKey) {
        const label = error instanceof Error ? error.message : error;
        const message = credentialErrorMessage(label) ?? I18N.settings.appleTranslationPrepareFailed;
        setPrepareError({ key: requestKey, message });
        notify(message, label !== "apple_translation_cancelled");
      }
    } finally {
      inFlight.current = false;
      if (mounted.current) { setPreparing(false); callbacks.current.onBusyChange?.(false); }
    }
  };
  const stopped = disabled || requiresStop || preparing;
  const problem = targetLanguage === "original" ? I18N.settings.appleTranslationChooseTarget
    : failed ? I18N.settings.appleTranslationStatusFailed
    : !support?.available || pair.status === "unavailable" ? I18N.settings.appleTranslationUnavailable
    : sameLanguage ? I18N.settings.appleTranslationSameLanguage
    : sourceLanguage === "auto" ? I18N.settings.appleTranslationChooseSource
    : !supportedPair || pair.status === "unsupported" ? I18N.settings.appleTranslationUnsupported
    : pair.failed ? I18N.settings.appleTranslationStatusFailed : null;
  return <div className="apple-translation-settings" aria-busy={loading || pair.loading || preparing}>
    {loading || pair.loading ? <span className="apple-speech-resource-status" role="status"><span className="settings-spinner" aria-hidden="true" />{I18N.settings.appleTranslationChecking}</span>
      : problem ? <InlineFeedback tone={failed || pair.failed ? "error" : "info"}>{problem}{(failed || pair.failed) && <button type="button" className="settings-link" disabled={stopped} onClick={() => void (failed ? onRetry() : pair.refresh())}>{I18N.settings.retryLoadingSettings}</button>}</InlineFeedback>
      : <div className="apple-speech-resource-actions">
        <span className="apple-speech-resource-status" role="status">{preparing ? <span className="settings-spinner" aria-hidden="true" /> : <Icon name={pair.status === "installed" ? "checkmark-circle" : "download"} />}{SOURCE_LANGUAGE_DISPLAY_NAMES[sourceLanguage]} → {TARGET_LANGUAGE_DISPLAY_NAMES[targetLanguage]} · {preparing ? I18N.settings.appleTranslationPreparing : pair.status === "installed" ? I18N.settings.appleTranslationReady : I18N.settings.appleTranslationNeedsSetup}</span>
        {pair.status === "supported" && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={stopped} onClick={() => void prepare()}><Icon name="download" />{I18N.settings.appleTranslationPrepare}</button>}
      </div>}
    {prepareError?.key === key && <InlineFeedback tone={prepareError.message === I18N.settings.appleTranslationCancelled ? "info" : "error"}>{prepareError.message}</InlineFeedback>}
    {requiresStop && !loading && !pair.loading && !problem && pair.status === "supported" && <InlineFeedback tone="info">{I18N.settings.appleLanguagePreparationRequiresStop}</InlineFeedback>}
  </div>;
}
