import { useEffect, useState, useSyncExternalStore } from "react";
import { Icon } from "../../../components/Icon";
import { effectiveUiLanguage, I18N } from "../../../lib/i18n";
import { appOpenReleases } from "../../../lib/ipc";
import { SettingsHelp } from "../feedback/SettingsHelp";
import { ReleaseNotes } from "./ReleaseNotes";
import { downloadPercent, isErrorState, updateInteraction, type UpdateCheckState } from "./softwareUpdateModel";
import { softwareUpdateSession } from "./softwareUpdateSession";
import type { SoftwareUpdater } from "./softwareUpdater";
import { useSettingsToast } from "../feedback/useSettingsToast";

/** Check only on General-page entry. The window-scoped session retains pending
 * operations when this view is hidden or remounted. Downloads remain explicit. */
export function SoftwareUpdate({ active = true, session = softwareUpdateSession }: {
  active?: boolean;
  session?: typeof softwareUpdateSession;
}) {
  const { state, currentVersion, platform, environment } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const [openingReleases, setOpeningReleases] = useState(false);
  const { beginToast } = useSettingsToast();
  useEffect(() => { if (active) void session.enter(); }, [active, session]);

  const manualDistribution = environment === "portable" || environment === "linuxPackage";
  const interaction = updateInteraction(state);
  const update = "update" in state ? state.update : undefined;
  const verifying = state.kind === "downloading" && state.transferComplete;
  const percent = verifying ? undefined : downloadPercent(state);
  const status = stateStatus(state);
  const failed = isErrorState(state);

  const handleAction = async () => {
    const notify = beginToast();
    const result = await session.performAction();
    if (result === "noUpdate") notify(I18N.settings.noUpdateAvailable);
    if (result === "restartRequested") notify(I18N.settings.restartRequested);
  };
  const openReleases = async () => {
    if (openingReleases) return;
    setOpeningReleases(true);
    const notify = beginToast();
    try { await appOpenReleases(); }
    catch { notify(I18N.settings.openUpdateFailed, true); }
    finally { setOpeningReleases(false); }
  };
  const releasesButton = (
    <button type="button" className="settings-button settings-button--quiet software-update__release-link"
      disabled={openingReleases} onClick={() => void openReleases()}>
      {openingReleases ? I18N.settings.openingUpdateRecovery : I18N.settings.openReleaseRecovery}
    </button>
  );

  return (
    <div className="software-update">
      <div className="software-update__header">
        <div className="software-update__copy">
          <div className="software-update__identity">
            <span className="settings-row__label">
              {I18N.settings.softwareUpdate}
              {manualDistribution && <SettingsHelp text={environment === "linuxPackage"
                ? I18N.settings.linuxPackageUpdateDescription : I18N.settings.portableUpdateDescription} label={I18N.settings.helpLabel} />}
            </span>
            <div className="software-update__versions">
              {currentVersion && <span className="software-update__current">
                <span className="settings-sr-only">{I18N.settings.currentVersion(currentVersion)}</span>
                <span aria-hidden="true">v{currentVersion}</span>
              </span>}
              {update && <>
                {currentVersion && <span className="software-update__arrow" aria-hidden="true">→</span>}
                <span className="software-update__target">
                  <span className="settings-sr-only">{I18N.settings.updateAvailable(update.version)}</span>
                  <span aria-hidden="true">v{update.version}</span>
                </span>
              </>}
            </div>
          </div>
        </div>
        <div className="software-update__actions">
          {manualDistribution ? releasesButton : (interaction.action || interaction.busy) && (
            <button type="button" className={`settings-button software-update-button ${interaction.emphasized ? "settings-button--primary" : "settings-button--quiet"}`}
              disabled={interaction.busy} aria-busy={interaction.busy} onClick={() => void handleAction()}>
              {interaction.busy && <span className="settings-spinner" aria-hidden="true" />}
              {actionLabel(state, platform)}
            </button>
          )}
        </div>
      </div>

      <div className={`software-update__status${failed ? " software-update__status--error" : ""}`} role="status" aria-live="polite" aria-atomic="true">
        {state.kind === "available" && <span className="settings-sr-only">{I18N.settings.updateAvailable(state.update.version)}</span>}
        {status && <span className="software-update__state">{status}</span>}
      </div>

      {state.kind === "downloading" && <div className="software-update__transfer">
        {!verifying && <div className="software-update__metrics" aria-hidden="true">
          <span>{percent === undefined
            ? I18N.settings.downloadingUnknown(formatBytes(state.downloadedBytes))
            : `${formatBytes(state.downloadedBytes)} / ${formatBytes(state.totalBytes ?? 0)}`}</span>
          {percent !== undefined && <span className="software-update__percent">{percent}%</span>}
        </div>}
        <div className="software-update__track">
          <progress className="software-update__progress" aria-label={verifying ? I18N.settings.verifyingUpdate : I18N.settings.downloadingUpdate}
            aria-valuetext={downloadStatus(state)} max={100}
            {...(percent === undefined ? {} : { value: percent })} />
          {percent === undefined
            ? <span className="software-update__activity" aria-hidden="true" />
            : <span className="software-update__fill" style={{ transform: `scaleX(${percent / 100})` }} aria-hidden="true" />}
        </div>
      </div>}

      {update && <div className="software-update__details">
        <details className="software-update__release" key={update.version}>
          <summary>{I18N.settings.releaseNotes}<Icon name="chevron-right" /></summary>
          <ReleaseNotes notes={update.notes} language={effectiveUiLanguage()} />
          {releasesButton}
        </details>
      </div>}
      {failed && !update && <div className="software-update__recovery">{releasesButton}</div>}
    </div>
  );
}

function actionLabel(state: UpdateCheckState, platform?: SoftwareUpdater["platform"]): string {
  switch (state.kind) {
    case "checking": return I18N.settings.checkingForUpdates;
    case "downloading": return state.transferComplete ? I18N.settings.verifyingUpdate : I18N.settings.downloadingUpdate;
    case "installing": return state.platform === "windows" ? I18N.settings.installingWindowsUpdate : I18N.settings.installingUpdate;
    case "restarting": return I18N.settings.restartingUpdate;
    case "downloadError": case "installError": case "restartError": case "checkError": return I18N.settings.retryUpdate;
    case "available": return I18N.settings.downloadUpdate;
    case "downloaded": return platform === "windows" ? I18N.settings.installAndRestartWindows : I18N.settings.installUpdate;
    case "restartReady": return I18N.settings.restartAndFinishUpdate;
    default: return I18N.settings.checkForUpdates;
  }
}

function stateStatus(state: UpdateCheckState): string {
  switch (state.kind) {
    case "noUpdate": return I18N.settings.noUpdateAvailable;
    case "downloaded": return I18N.settings.updateDownloadVerified;
    case "restartReady": return I18N.settings.updateReadyToRestart;
    case "downloading": {
      return state.transferComplete ? I18N.settings.verifyingUpdate : "";
    }
    case "restartRequested": return I18N.settings.restartRequested;
    case "windowsInstallerStarted": return I18N.settings.windowsInstallerStarted;
    case "checkError": return I18N.settings.updateCheckFailed;
    case "downloadError": return I18N.settings.updateDownloadFailed;
    case "installError": return I18N.settings.updateInstallFailed;
    case "restartError": return I18N.settings.updateRestartFailed;
    default: return "";
  }
}

function downloadStatus(state: Extract<UpdateCheckState, { kind: "downloading" }>): string {
  if (state.transferComplete) return I18N.settings.verifyingUpdate;
  const percent = downloadPercent(state);
  return percent === undefined
    ? I18N.settings.downloadingUnknown(formatBytes(state.downloadedBytes))
    : I18N.settings.downloadingKnown(percent, formatBytes(state.downloadedBytes), formatBytes(state.totalBytes ?? 0));
}

function formatBytes(bytes: number): string {
  if (bytes < 1_024) return `${Math.max(0, bytes)} B`;
  if (bytes < 1_048_576) return `${(bytes / 1_024).toFixed(1)} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}
