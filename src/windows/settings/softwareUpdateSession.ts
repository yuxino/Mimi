import { createUpdaterForEnvironment } from "./softwareUpdateEnvironment";
import {
  applyDownloadEvent,
  normalizeReleaseNotes,
  updateInteraction,
  withRecoveryStatus,
  type UpdateCheckState,
} from "./softwareUpdateModel";
import type {
  SoftwareUpdater,
  UpdateCandidate,
  UpdaterPlatform,
} from "./softwareUpdater";

export const AUTOMATIC_UPDATE_CHECK_INTERVAL_MS = 15 * 60 * 1_000;

type UpdateEnvironment = Awaited<ReturnType<typeof createUpdaterForEnvironment>>;
type ActionResult = "noUpdate" | "restartRequested" | undefined;

export interface SoftwareUpdateSnapshot {
  state: UpdateCheckState;
  currentVersion?: string;
  platform?: UpdaterPlatform;
  environment: "uninitialized" | UpdateEnvironment["kind"];
}

export interface SoftwareUpdateSession {
  getSnapshot(): SoftwareUpdateSnapshot;
  subscribe(listener: () => void): () => void;
  enter(): Promise<void>;
  performAction(): Promise<ActionResult>;
  setRecoveryStatus(status: "idle" | "opening" | "error"): void;
}

interface SessionOptions {
  createEnvironment?: () => Promise<UpdateEnvironment>;
  now?: () => number;
  cooldownMs?: number;
}

/** One settings-window session owns the native candidate and ongoing operation.
 * Subscribing is presentation only: leaving General must not dispose a verified
 * download or cause a StrictMode remount to make another request. */
export function createSoftwareUpdateSession({
  createEnvironment = createUpdaterForEnvironment,
  now = Date.now,
  cooldownMs = AUTOMATIC_UPDATE_CHECK_INTERVAL_MS,
}: SessionOptions = {}): SoftwareUpdateSession {
  let snapshot: SoftwareUpdateSnapshot = {
    state: { kind: "idle" },
    environment: "uninitialized",
  };
  const listeners = new Set<() => void>();
  let updater: SoftwareUpdater | undefined;
  let candidate: UpdateCandidate | undefined;
  let operation: Promise<ActionResult> | undefined;
  let lastCheckAt: number | undefined;

  function publish(next: SoftwareUpdateSnapshot) {
    snapshot = next;
    listeners.forEach((listener) => listener());
  }

  function setState(state: UpdateCheckState) {
    publish({ ...snapshot, state });
  }

  function startOperation(
    state: UpdateCheckState,
    run: () => Promise<ActionResult>,
  ): Promise<ActionResult> {
    // Lock before notifying subscribers, including synchronous/reentrant ones.
    const pending = Promise.resolve()
      .then(run)
      .finally(() => {
        operation = undefined;
      });
    operation = pending;
    setState(state);
    return pending;
  }

  function check(): Promise<ActionResult> {
    // Failed environment detection counts too: repeatedly entering General must
    // not retry a broken native bridge or package-mode probe without a cooldown.
    lastCheckAt = now();
    return startOperation({ kind: "checking" }, async () => {
      try {
        if (!updater) {
          const environment = await createEnvironment();
          if (environment.kind !== "installed") {
            publish({
              state: { kind: "idle" },
              environment: environment.kind,
              currentVersion: environment.currentVersion,
            });
            return;
          }
          updater = environment.updater;
          publish({
            ...snapshot,
            environment: "installed",
            currentVersion: updater.currentVersion,
            platform: updater.platform,
          });
        }
        const available = await updater.check();
        if (!available) {
          setState({ kind: "noUpdate" });
          return "noUpdate";
        }
        candidate = available;
        setState({
          kind: "available",
          update: {
            version: available.version,
            notes: normalizeReleaseNotes(available.notes),
          },
        });
      } catch {
        // Keep details out of presentation: native errors may contain paths or
        // remote response data. Both initialization and check failures can retry.
        setState({ kind: "checkError", recovery: "idle" });
      }
    });
  }

  return {
    getSnapshot: () => snapshot,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    async enter() {
      if (operation) {
        await operation;
        return;
      }
      if (
        snapshot.environment === "portable" ||
        snapshot.environment === "linuxPackage" ||
        "update" in snapshot.state ||
        (lastCheckAt !== undefined && now() - lastCheckAt < cooldownMs)
      ) {
        return;
      }
      // Automatic checks produce no notification events for the view.
      await check();
    },
    performAction() {
      if (operation) return Promise.resolve(undefined);
      const action = updateInteraction(snapshot.state).action;
      if (
        !action ||
        snapshot.environment === "portable" ||
        snapshot.environment === "linuxPackage"
      ) {
        return Promise.resolve(undefined);
      }
      if (action === "check") return check();

      const activeCandidate = candidate;
      const activeUpdater = updater;
      const state = snapshot.state;
      if (!activeCandidate || !activeUpdater || !("update" in state)) {
        return Promise.resolve(undefined);
      }
      const { update } = state;

      if (action === "download") {
        return startOperation(
          { kind: "downloading", update, downloadedBytes: 0 },
          async () => {
            try {
              await activeCandidate.download((event) => {
                setState(applyDownloadEvent(snapshot.state, event));
              });
              // Finished is only a transfer event. Only a successful download
              // promise confirms the native updater's signature verification.
              setState({ kind: "downloaded", update });
            } catch {
              setState({ kind: "downloadError", update, recovery: "idle" });
            }
            return undefined;
          },
        );
      }

      if (action === "install") {
        return startOperation(
          { kind: "installing", update, platform: activeUpdater.platform },
          async () => {
            try {
              await activeCandidate.install();
              setState(
                activeUpdater.platform === "windows"
                  ? { kind: "windowsInstallerStarted", update }
                  : { kind: "restartReady", update },
              );
            } catch {
              setState({ kind: "installError", update, recovery: "idle" });
            }
            return undefined;
          },
        );
      }

      return startOperation({ kind: "restarting", update }, async () => {
        try {
          await activeUpdater.relaunch();
          setState({ kind: "restartRequested", update });
          return "restartRequested";
        } catch {
          setState({ kind: "restartError", update, recovery: "idle" });
        }
      });
    },
    setRecoveryStatus(status) {
      setState(withRecoveryStatus(snapshot.state, status));
    },
  };
}

export const softwareUpdateSession = createSoftwareUpdateSession();
