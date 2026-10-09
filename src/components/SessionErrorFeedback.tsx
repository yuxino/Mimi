import { useState } from "react";
import { appOpenAudioPrivacySettings } from "../lib/ipc";
import { systemAudioPermissionCopy } from "../lib/systemAudioPermissions";
import { Icon } from "./Icon";
import { I18N } from "../lib/i18n";
import "./session-error-feedback.css";

interface SessionErrorFeedbackProps {
  /** A safe, localized message from the session error selector. */
  message: string;
  /** A safe short cause for the subtitle canvas; omit for detailed surfaces. */
  summary?: string;
  /** The open control panel owns the prominent recovery actions. */
  actionsHidden?: boolean;
  onConfigure: () => void;
  configureLabel?: string;
  onRetry?: () => void;
  disabled?: boolean;
  permissionRequired?: boolean;
}

/** Ongoing failures stay readable beside their recovery actions. */
export function SessionErrorFeedback({ message, summary, actionsHidden = false, onConfigure, configureLabel = I18N.settings.openSpeechSettings, onRetry, disabled = false, permissionRequired = false }: SessionErrorFeedbackProps) {
  const compact = summary !== undefined;
  const [opening, setOpening] = useState(false);
  const [openFailed, setOpenFailed] = useState(false);
  const permission = systemAudioPermissionCopy();
  const openPermissions = async () => {
    if (opening) return;
    setOpening(true);
    setOpenFailed(false);
    try { await appOpenAudioPrivacySettings(); }
    catch { setOpenFailed(true); }
    finally { setOpening(false); }
  };
  return <div className={`session-error-feedback${compact ? " session-error-feedback--summary" : ""}`} role="alert">
    <p className="session-error-feedback__message">{summary ?? message}</p>
    {openFailed && permissionRequired && <p className="session-error-feedback__message">{permission.failed}</p>}
    {!actionsHidden && <div className="session-error-feedback__actions">
      {permissionRequired && <button type="button" className="session-error-feedback__action" disabled={disabled || opening} onClick={() => { void openPermissions(); }}>
        <Icon name="gear" />{permission.open}
      </button>}
      {!permissionRequired && (!compact || !onRetry) && <button type="button" className="session-error-feedback__action" disabled={disabled} onClick={onConfigure}>
        <Icon name="gear" />{configureLabel}
      </button>}
      {onRetry && <button type="button" className="session-error-feedback__action" disabled={disabled} onClick={onRetry}>
        <Icon name="play" />{I18N.settings.sessionRetry}
      </button>}
    </div>}
  </div>;
}
