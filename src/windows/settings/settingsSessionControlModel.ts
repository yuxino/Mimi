import type { CredentialState, SessionStateEvent } from "../../lib/types";

type SettingsSessionStatusKind = SessionStateEvent["status"]["kind"];

export type SettingsSessionPendingAction = "start" | "stop" | null;

export type SettingsSessionVisibleStatus =
  | "idle"
  | "connecting"
  | "listening"
  | "paused"
  | "stopping"
  | "error"
  | "setupRequired"
  | "credentialUnavailable";

interface SettingsSessionControlInput {
  statusKind: SettingsSessionStatusKind;
  isActive: boolean;
  isPaused: boolean;
  credentialState: CredentialState;
  pendingAction: SettingsSessionPendingAction;
}

interface SettingsSessionControlState {
  checked: boolean;
  disabled: boolean;
  canConfigure: boolean;
  visibleStatus: SettingsSessionVisibleStatus;
}

interface SettingsSessionNativeState {
  statusKind: SettingsSessionStatusKind;
  isActive: boolean;
}

/**
 * Owns the click-side pending action until the native session event confirms
 * the requested state. IPC completion can precede that event, so a successful
 * command must not make the switch interactive again by itself.
 */
export class SettingsSessionActionCoordinator {
  #pendingAction: SettingsSessionPendingAction = null;

  get pendingAction(): SettingsSessionPendingAction {
    return this.#pendingAction;
  }

  begin(action: Exclude<SettingsSessionPendingAction, null>): boolean {
    if (this.#pendingAction !== null) return false;
    this.#pendingAction = action;
    return true;
  }

  commandRejected(
    action: Exclude<SettingsSessionPendingAction, null>,
  ): boolean {
    if (this.#pendingAction !== action) return false;
    this.#pendingAction = null;
    return true;
  }

  /** Applies a fresh native transition; callers must not replay the old snapshot. */
  observeNativeState({
    statusKind,
    isActive,
  }: SettingsSessionNativeState): SettingsSessionPendingAction {
    if (
      (this.#pendingAction === "start" &&
        ((isActive &&
          (statusKind === "connecting" || statusKind === "listening")) ||
          (!isActive &&
            (statusKind === "idle" || statusKind === "error")))) ||
      (this.#pendingAction === "stop" && !isActive)
    ) {
      this.#pendingAction = null;
    }
    return this.#pendingAction;
  }
}

/** Derives the settings switch from native state plus the in-flight click. */
export function settingsSessionControlState({
  statusKind,
  isActive,
  isPaused,
  credentialState,
  pendingAction,
}: SettingsSessionControlInput): SettingsSessionControlState {
  const checked =
    pendingAction === "start" || (pendingAction !== "stop" && isActive);
  const isChanging =
    pendingAction !== null ||
    statusKind === "connecting" ||
    statusKind === "stopping";
  const needsCredential = !isActive && credentialState !== "present";

  let visibleStatus: SettingsSessionVisibleStatus;
  if (pendingAction === "start" || statusKind === "connecting") {
    visibleStatus = "connecting";
  } else if (pendingAction === "stop" || statusKind === "stopping") {
    visibleStatus = "stopping";
  } else if (!isActive && credentialState === "missing") {
    visibleStatus = "setupRequired";
  } else if (!isActive && credentialState === "unavailable") {
    visibleStatus = "credentialUnavailable";
  } else if (statusKind === "error") {
    visibleStatus = "error";
  } else if (isActive && isPaused) {
    visibleStatus = "paused";
  } else if (isActive || statusKind === "listening") {
    visibleStatus = "listening";
  } else {
    visibleStatus = "idle";
  }

  return {
    checked,
    disabled: isChanging || needsCredential,
    canConfigure: !isActive && credentialState !== "present",
    visibleStatus,
  };
}
