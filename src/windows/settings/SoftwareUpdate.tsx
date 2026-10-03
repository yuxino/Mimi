import { useEffect, useRef, useState } from "react";
import { I18N } from "../../lib/i18n";
import { appOpenReleases } from "../../lib/ipc";
import { InlineFeedback, SettingsRow } from "./SettingsPrimitives";
import {
  applyDownloadEvent,
  downloadPercent,
  isErrorState,
  normalizeReleaseNotes,
  updateInteraction,
  withRecoveryStatus,
  type AvailableUpdate,
  type UpdateCheckState,
} from "./softwareUpdateModel";
import type { SoftwareUpdater, UpdateCandidate } from "./softwareUpdater";
import { createUpdaterForEnvironment } from "./softwareUpdateEnvironment";

/** User-initiated signed updater. It never polls, downloads, or installs in the
 * background. */
export function SoftwareUpdate() {
  const [currentVersion, setCurrentVersion] = useState<string>();
  const [updater, setUpdater] = useState<SoftwareUpdater>();
  const [portable, setPortable] = useState(false);
  const [linuxPackage, setLinuxPackage] = useState(false);
  const [openingReleases, setOpeningReleases] = useState(false);
  const [portableOpenError, setPortableOpenError] = useState(false);
  const [state, setState] = useState<UpdateCheckState>({ kind: "idle" });
  const candidateRef = useRef<UpdateCandidate | undefined>(undefined);
  const operationRef = useRef(false);

  useEffect(() => {
    let disposed = false;

    void createUpdaterForEnvironment()
      .then((environment) => {
        if (disposed) return;
        if (environment.kind !== "installed") {
          setPortable(environment.kind === "portable");
          setLinuxPackage(environment.kind === "linuxPackage");
          setCurrentVersion(environment.currentVersion);
        } else {
          setUpdater(environment.updater);
          setCurrentVersion(environment.updater.currentVersion);
        }
      })
      .catch(() => {
        if (!disposed) {
          setState({ kind: "checkError", recovery: "idle" });
        }
      });

    return () => {
      disposed = true;
    };
  }, []);

  const interaction = updateInteraction(state);

  const handleAction = async () => {
    if (!updater || !interaction.action || operationRef.current) return;
    operationRef.current = true;

    try {
      if (interaction.action === "check") {
        await candidateRef.current?.close().catch(() => {});
        candidateRef.current = undefined;
        setState({ kind: "checking" });
        try {
          const candidate = await updater.check();
          if (!candidate) {
            setState({ kind: "noUpdate" });
            return;
          }

          candidateRef.current = candidate;
          setState({
            kind: "available",
            update: candidateMetadata(candidate),
          });
        } catch {
          setState({ kind: "checkError", recovery: "idle" });
        }
        return;
      }

      const candidate = candidateRef.current;
      if (!candidate || !("update" in state)) return;
      const update = state.update;

      if (interaction.action === "download") {
        setState({
          kind: "downloading",
          update,
          downloadedBytes: 0,
        });
        try {
          await candidate.download((event) => {
            setState((current) => applyDownloadEvent(current, event));
          });
          setState({ kind: "downloaded", update });
        } catch {
          setState({ kind: "downloadError", update, recovery: "idle" });
        }
        return;
      }

      if (interaction.action === "install") {
        setState({ kind: "installing", update, platform: updater.platform });
        try {
          await candidate.install();
          setState(
            updater.platform === "windows"
              ? { kind: "windowsInstallerStarted", update }
              : { kind: "restartReady", update },
          );
        } catch {
          setState({ kind: "installError", update, recovery: "idle" });
        }
        return;
      }

      setState({ kind: "restarting", update });
      try {
        await updater.relaunch();
        setState({ kind: "restartRequested", update });
      } catch {
        setState({ kind: "restartError", update, recovery: "idle" });
      }
    } finally {
      operationRef.current = false;
    }
  };

  const handleRecovery = async () => {
    if (!isErrorState(state) || state.recovery === "opening") return;
    setState((current) => withRecoveryStatus(current, "opening"));
    try {
      await appOpenReleases();
      setState((current) => withRecoveryStatus(current, "idle"));
    } catch {
      setState((current) => withRecoveryStatus(current, "error"));
    }
  };

  const busy = interaction.busy || !updater;

  if (portable || linuxPackage) {
    return (
      <div className="software-update">
        <SettingsRow
          label={I18N.settings.softwareUpdate}
          description={
            currentVersion
              ? I18N.settings.currentVersion(currentVersion)
              : I18N.settings.updateDescription
          }
          align="start"
        >
          <button
            type="button"
            className="settings-button settings-button--quiet software-update-button"
            disabled={openingReleases}
            onClick={() => {
              setOpeningReleases(true);
              setPortableOpenError(false);
              void appOpenReleases()
                .catch(() => setPortableOpenError(true))
                .finally(() => setOpeningReleases(false));
            }}
          >
            {openingReleases
              ? I18N.settings.openingUpdateRecovery
              : I18N.settings.openReleaseRecovery}
          </button>
        </SettingsRow>
        <span className="software-update-live-status" role="status">
          {linuxPackage ? I18N.settings.linuxPackageUpdateDescription : I18N.settings.portableUpdateDescription}
        </span>
        {portableOpenError && <span role="alert">{I18N.settings.openUpdateFailed}</span>}
      </div>
    );
  }

  return (
    <div className="software-update">
      <SettingsRow
        label={I18N.settings.softwareUpdate}
        description={
          currentVersion
            ? I18N.settings.currentVersion(currentVersion)
            : I18N.settings.updateDescription
        }
        align="start"
      >
        {interaction.action && (
          <button
            type="button"
            className={`settings-button software-update-button ${
              interaction.emphasized
                ? "settings-button--primary"
                : "settings-button--quiet"
            }`}
            disabled={busy}
            aria-busy={interaction.busy}
            onClick={() => void handleAction()}
          >
            {actionLabel(state, updater?.platform)}
          </button>
        )}
      </SettingsRow>

      <span
        className="software-update-live-status"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {stateStatus(state)}
      </span>

      <UpdateDetails state={state} onRecovery={handleRecovery} />
    </div>
  );
}

function candidateMetadata(candidate: UpdateCandidate): AvailableUpdate {
  return {
    version: candidate.version,
    notes: normalizeReleaseNotes(candidate.notes),
  };
}

function actionLabel(
  state: UpdateCheckState,
  platform?: SoftwareUpdater["platform"],
): string {
  switch (state.kind) {
    case "checking":
      return I18N.settings.checkingForUpdates;
    case "downloading":
      return I18N.settings.downloadingUpdate;
    case "installing":
      return state.platform === "windows"
        ? I18N.settings.installingWindowsUpdate
        : I18N.settings.installingUpdate;
    case "restarting":
      return I18N.settings.restartingUpdate;
    case "downloadError":
    case "installError":
    case "restartError":
    case "checkError":
      return I18N.settings.retryUpdate;
    case "available":
      return I18N.settings.downloadUpdate;
    case "downloaded":
      return platform === "windows"
        ? I18N.settings.installAndRestartWindows
        : I18N.settings.installUpdate;
    case "restartReady":
      return I18N.settings.restartAndFinishUpdate;
    default:
      return I18N.settings.checkForUpdates;
  }
}

function stateStatus(state: UpdateCheckState): string {
  switch (state.kind) {
    case "checking":
      return I18N.settings.checkingForUpdates;
    case "downloading": {
      const percent = downloadPercent(state);
      return percent === undefined
        ? I18N.settings.downloadingUnknown(formatBytes(state.downloadedBytes))
        : I18N.settings.downloadingKnown(
            percent,
            formatBytes(state.downloadedBytes),
            formatBytes(state.totalBytes ?? 0),
          );
    }
    case "installing":
      return state.platform === "windows"
        ? I18N.settings.installingWindowsUpdate
        : I18N.settings.installingUpdate;
    case "restarting":
      return I18N.settings.restartingUpdate;
    case "restartRequested":
      return I18N.settings.restartRequested;
    case "windowsInstallerStarted":
      return I18N.settings.windowsInstallerStarted;
    default:
      return "";
  }
}

function UpdateDetails({
  state,
  onRecovery,
}: {
  state: UpdateCheckState;
  onRecovery: () => Promise<void>;
}) {
  const update = "update" in state ? state.update : undefined;
  const percent = downloadPercent(state);

  return (
    <div className="software-update__details">
      {update && (
        <div className="software-update__release">
          <p className="software-update__version">
            {I18N.settings.updateAvailable(update.version)}
          </p>
          <p className="software-update__notes-label">
            {I18N.settings.releaseNotes}
          </p>
          <p className="software-update__notes">
            {update.notes || I18N.settings.noReleaseNotes}
          </p>
        </div>
      )}

      {state.kind === "downloading" && (
        <div className="software-update__progress">
          <progress
            aria-label={I18N.settings.downloadingUpdate}
            {...(percent === undefined ? {} : { value: percent, max: 100 })}
          />
          <span>{stateStatus(state)}</span>
        </div>
      )}

      <UpdateFeedback state={state} />

      {isErrorState(state) && (
        <div className="software-update__recovery">
          <button
            type="button"
            className="settings-button settings-button--quiet"
            disabled={state.recovery === "opening"}
            onClick={() => void onRecovery()}
          >
            {state.recovery === "opening"
              ? I18N.settings.openingUpdateRecovery
              : I18N.settings.openReleaseRecovery}
          </button>
          {state.recovery === "error" && (
            <span role="alert">{I18N.settings.openUpdateFailed}</span>
          )}
        </div>
      )}
    </div>
  );
}

function UpdateFeedback({ state }: { state: UpdateCheckState }) {
  switch (state.kind) {
    case "noUpdate":
      return (
        <InlineFeedback tone="success">
          {I18N.settings.noUpdateAvailable}
        </InlineFeedback>
      );
    case "downloaded":
      return (
        <InlineFeedback tone="success">
          {I18N.settings.updateDownloadVerified}
        </InlineFeedback>
      );
    case "restartReady":
      return (
        <InlineFeedback tone="success">
          {I18N.settings.restartAndFinishUpdate}
        </InlineFeedback>
      );
    case "restartRequested":
      return (
        <InlineFeedback tone="success">
          {I18N.settings.restartRequested}
        </InlineFeedback>
      );
    case "windowsInstallerStarted":
      return (
        <InlineFeedback tone="info">
          {I18N.settings.windowsInstallerStarted}
        </InlineFeedback>
      );
    case "checkError":
      return (
        <InlineFeedback tone="error">
          {I18N.settings.updateCheckFailed}
        </InlineFeedback>
      );
    case "downloadError":
      return (
        <InlineFeedback tone="error">
          {I18N.settings.updateDownloadFailed}
        </InlineFeedback>
      );
    case "installError":
      return (
        <InlineFeedback tone="error">
          {I18N.settings.updateInstallFailed}
        </InlineFeedback>
      );
    case "restartError":
      return (
        <InlineFeedback tone="error">
          {I18N.settings.updateRestartFailed}
        </InlineFeedback>
      );
    default:
      return null;
  }
}

function formatBytes(bytes: number): string {
  if (bytes < 1_024) return `${Math.max(0, bytes)} B`;
  if (bytes < 1_048_576) return `${(bytes / 1_024).toFixed(1)} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}
