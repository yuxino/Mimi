import { Switch } from "../../components/Switch";
import { SessionErrorFeedback } from "../../components/SessionErrorFeedback";
import { I18N } from "../../lib/i18n";
import type { DesktopShortcutCommands } from "../../lib/ipc";
import type { SettingsSessionVisibleStatus } from "./settingsSessionControlModel";

interface SettingsSessionControlsProps {
  checked: boolean;
  disabled: boolean;
  status: SettingsSessionVisibleStatus;
  statusText: string;
  errorMessage?: string | null;
  permissionRequired?: boolean;
  errorRequiresConfiguration?: boolean;
  isActive: boolean;
  isChanging: boolean;
  immersive: boolean;
  canConfigure: boolean;
  actionFailed: boolean;
  nativeShortcuts: boolean;
  desktopShortcuts: DesktopShortcutCommands | null | undefined;
  compact?: boolean;
  retrying?: boolean;
  resuming?: boolean;
  resumeFailed?: boolean;
  resumeFailureMessage?: string | null;
  onSessionChange: (enabled: boolean) => void;
  onResume: () => void;
  onImmersiveChange: (enabled: boolean) => void;
  onConfigure: () => void;
  configureLabel?: string;
}

export function SettingsSessionControls(props: SettingsSessionControlsProps) {
  const isMac = /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent);
  // An existing immersive preference must always be reversible, even after
  // stopping or losing credentials. Enabling it never starts a session.
  const immersiveDisabled = !props.immersive && (!props.isActive || props.isChanging);
  return <section className={`settings-session-card${props.compact ? " settings-session-card--compact" : ""}`} aria-labelledby="settings-session-title">
    <div className="settings-session-control">
      <div className="settings-session-control__copy">
        <div className="settings-session-control__heading">
          <h2 id="settings-session-title">{I18N.settings.liveSubtitles}</h2>
          {props.nativeShortcuts && <kbd aria-label={I18N.settings.startStopShortcut}>{isMac ? "⌘⇧Space" : "Ctrl+Shift+Space"}</kbd>}
        </div>
        <span id="settings-session-status" className="settings-session-status" data-status={props.status} aria-live="polite">
          <span aria-hidden="true" />{props.statusText}
        </span>
        {props.canConfigure && !props.errorMessage && <button type="button" className="settings-button settings-button--quiet settings-button--compact" onClick={props.onConfigure}>{I18N.settings.configureService}</button>}
      </div>
      <div className="settings-session-control__actions">
        {((props.status === "error" && !props.errorRequiresConfiguration) || props.retrying) && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={props.disabled} onClick={() => props.onSessionChange(true)}>{props.retrying ? I18N.settings.sessionConnecting : I18N.settings.sessionRetry}</button>}
        {(props.status === "paused" || props.resuming) && <button type="button" className="settings-button settings-button--compact settings-session-resume" disabled={props.disabled || props.resuming || props.isChanging} aria-busy={props.resuming || undefined} aria-describedby="settings-session-status" onClick={props.onResume}>
          {props.resuming && <span className="settings-session-resume__busy" aria-hidden="true" />}
          {props.resuming ? I18N.settings.sessionResuming : I18N.settings.sessionResume}
        </button>}
        <Switch checked={props.checked} disabled={props.disabled} aria-label={I18N.settings.liveSubtitles} aria-describedby="settings-session-status" onChange={enabled => enabled && props.errorRequiresConfiguration ? props.onConfigure() : props.onSessionChange(enabled)} />
      </div>
    </div>
    {props.errorMessage && <SessionErrorFeedback permissionRequired={props.permissionRequired} message={props.errorMessage} onConfigure={props.onConfigure} configureLabel={props.configureLabel} disabled={props.isChanging} />}
    {!props.compact && <div className="settings-session-control">
      <div className="settings-session-control__copy">
        <div className="settings-session-control__heading">
          <h3>{I18N.settings.blendBackground}</h3>
          {props.nativeShortcuts && <kbd>{isMac ? "⌘⇧M" : "Ctrl+Shift+M"}</kbd>}
        </div>
        <p id="settings-immersive-help">{immersiveDisabled && !props.isActive ? I18N.settings.immersiveStartFirst : I18N.settings.blendBackgroundHelp}</p>
      </div>
      <Switch checked={props.immersive} disabled={immersiveDisabled} aria-label={I18N.settings.blendBackground} aria-describedby="settings-immersive-help" onChange={props.onImmersiveChange} />
    </div>}
    {props.desktopShortcuts && <details className="settings-session-help settings-desktop-shortcuts">
      <summary>{I18N.settings.systemShortcutSetup}</summary>
      <p>{I18N.settings.systemShortcutInstructions}</p>
      <dl>
        <dt>{I18N.settings.startStopShortcut}</dt><dd><code>{props.desktopShortcuts.toggleSession}</code></dd>
        <dt>{I18N.tray.blendBackground}</dt><dd><code>{props.desktopShortcuts.toggleImmersive}</code></dd>
        <dt>{I18N.settings.subtitleDisplay}</dt><dd><code>{props.desktopShortcuts.cycleSubtitleDisplay}</code></dd>
      </dl>
    </details>}
    {((props.actionFailed && !props.permissionRequired) || props.resumeFailed) && <p className="settings-feedback" data-tone="error" role="alert">{props.resumeFailed ? props.resumeFailureMessage ?? I18N.settings.sessionResumeFailed : I18N.settings.sessionActionFailed}</p>}
  </section>;
}
