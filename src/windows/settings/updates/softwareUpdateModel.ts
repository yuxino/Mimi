import { RELEASE_NOTES_SOURCE_LIMIT } from "./releaseNotesModel";
import type {
  UpdateDownloadEvent,
  UpdaterPlatform,
} from "./softwareUpdater";

export interface AvailableUpdate {
  version: string;
  notes: string;
}

type RecoveryStatus = "idle" | "opening" | "error";

export type UpdateCheckState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "noUpdate" }
  | { kind: "available"; update: AvailableUpdate }
  | {
      kind: "downloading";
      update: AvailableUpdate;
      downloadedBytes: number;
      totalBytes?: number;
      transferComplete?: boolean;
    }
  | { kind: "downloaded"; update: AvailableUpdate }
  | { kind: "installing"; update: AvailableUpdate; platform: UpdaterPlatform }
  | { kind: "restartReady"; update: AvailableUpdate }
  | { kind: "restarting"; update: AvailableUpdate }
  | { kind: "restartRequested"; update: AvailableUpdate }
  | { kind: "windowsInstallerStarted"; update: AvailableUpdate }
  | { kind: "checkError"; recovery: RecoveryStatus }
  | { kind: "downloadError"; update: AvailableUpdate; recovery: RecoveryStatus }
  | { kind: "installError"; update: AvailableUpdate; recovery: RecoveryStatus }
  | { kind: "restartError"; update: AvailableUpdate; recovery: RecoveryStatus };

export type UpdateAction = "check" | "download" | "install" | "restart";

export interface UpdateInteraction {
  action?: UpdateAction;
  busy: boolean;
  emphasized: boolean;
}

export function updateInteraction(state: UpdateCheckState): UpdateInteraction {
  switch (state.kind) {
    case "checking":
    case "downloading":
    case "installing":
    case "restarting":
      return { busy: true, emphasized: true };
    case "available":
    case "downloadError":
      return { action: "download", busy: false, emphasized: true };
    case "downloaded":
    case "installError":
      return { action: "install", busy: false, emphasized: true };
    case "restartReady":
    case "restartError":
      return { action: "restart", busy: false, emphasized: true };
    case "idle":
    case "noUpdate":
    case "checkError":
      return { action: "check", busy: false, emphasized: false };
    case "restartRequested":
    case "windowsInstallerStarted":
      return { busy: false, emphasized: false };
  }
}

export function applyDownloadEvent(
  state: UpdateCheckState,
  event: UpdateDownloadEvent,
): UpdateCheckState {
  if (state.kind !== "downloading") return state;

  switch (event.event) {
    case "Started":
      return {
        ...state,
        totalBytes: validTotalBytes(event.data.contentLength),
      };
    case "Progress":
      return {
        ...state,
        downloadedBytes:
          state.downloadedBytes + Math.max(0, event.data.chunkLength),
      };
    case "Finished":
      return { ...state, transferComplete: true };
  }
}

export function downloadPercent(state: UpdateCheckState): number | undefined {
  if (state.kind !== "downloading" || state.totalBytes === undefined) {
    return undefined;
  }
  return Math.min(
    100,
    Math.max(0, Math.floor((state.downloadedBytes / state.totalBytes) * 100)),
  );
}

export function normalizeReleaseNotes(notes?: string): string {
  const normalized = notes?.replace(/\r\n/g, "\n").trim() ?? "";
  return normalized.slice(0, RELEASE_NOTES_SOURCE_LIMIT);
}

export function isErrorState(
  state: UpdateCheckState,
): state is Extract<UpdateCheckState, { recovery: RecoveryStatus }> {
  return "recovery" in state;
}

export function withRecoveryStatus(
  state: UpdateCheckState,
  recovery: RecoveryStatus,
): UpdateCheckState {
  return isErrorState(state) ? { ...state, recovery } : state;
}

function validTotalBytes(value?: number): number | undefined {
  return value !== undefined && Number.isFinite(value) && value > 0
    ? value
    : undefined;
}
