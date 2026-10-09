import { isSystemAudioPermissionDenied } from "../../lib/systemAudioPermissions";
import { useCallback, useEffect, useRef, useState } from "react";
import type { SettingsNavigationTarget } from "../../lib/ipc";
import { I18N } from "../../lib/i18n";
import { sessionErrorSettingsTarget } from "../../lib/connectionDiagnostics";
import { credentialStateForTarget } from "../../lib/providerCapabilities";
import { useDesktopShortcuts } from "../../lib/useDesktopShortcuts";
import { selectSessionErrorMessage, selectSessionStatusKind, useStore } from "../../lib/store";
import { useSessionAction } from "../overlay/useSessionAction";
import { SettingsSessionControls } from "./SettingsSessionControls";
import { useSettingsToast } from "./useSettingsToast";
import { SettingsSessionActionCoordinator, settingsSessionControlState, type SettingsSessionPendingAction, type SettingsSessionVisibleStatus } from "./settingsSessionControlModel";

export function SubtitleSessionControls({ visible = true, compact = false, onConfigure }: { visible?: boolean; compact?: boolean; onConfigure: (target?: SettingsNavigationTarget) => void }) {
  const { nativeShortcuts, commands } = useDesktopShortcuts();
  const sessionStatusKind = useStore(selectSessionStatusKind);
  const sessionErrorMessage = useStore(selectSessionErrorMessage);
  const permissionRequired = useStore(state => state.session.status.kind === "error" && isSystemAudioPermissionDenied(state.session.status.message));
  const errorSettingsTarget = useStore(state => state.session.status.kind === "error" ? sessionErrorSettingsTarget(state.session.status.message) : null);
  const errorRequiresConfiguration = errorSettingsTarget !== null;
  const sessionIsActive = useStore(state => state.session.isActive);
  const sessionIsPaused = useStore(state => state.session.isPaused);
  const translationRecoveryReason = useStore(state => state.session.translationRecovery?.reason);
  const translationRecoveryRetryScheduled = useStore(state => state.session.translationRecovery?.retryScheduled);
  const settings = useStore(state => state.settings);
  const start = useStore(state => state.start), stop = useStore(state => state.stop);
  const togglePaused = useStore(state => state.togglePaused);
  const saveSettings = useStore(state => state.saveSettings);
  const { runWithToast } = useSettingsToast();
  const activeProfile = settings.profiles.find(profile => profile.id === settings.activeProfileId);
  const [sessionPendingAction, setSessionPendingAction] = useState<SettingsSessionPendingAction>(null);
  const [sessionActionError, setSessionActionError] = useState(false);
  const [sessionActionCoordinator] = useState(() => new SettingsSessionActionCoordinator());
  const resumeInFlight = useRef(false);
  const { pending: sessionIsResuming, failed: sessionResumeFailed, failureMessage: sessionResumeFailureMessage, run: runResume, clearFailure: clearResumeFailure } = useSessionAction();
  const isChangingSession = sessionStatusKind === "connecting" || sessionStatusKind === "stopping";
  const sessionControl = settingsSessionControlState({ statusKind: sessionStatusKind, isActive: sessionIsActive, isPaused: sessionIsPaused,
    credentialState: credentialStateForTarget(activeProfile, settings.targetLanguage), pendingAction: sessionPendingAction });
  const changeSession = useCallback(
    (checked: boolean) => {
      const pendingAction: Exclude<SettingsSessionPendingAction, null> = checked ? "start" : "stop";
      if (resumeInFlight.current || !sessionActionCoordinator.begin(pendingAction)) {
        return;
      }
      setSessionPendingAction(pendingAction);
      setSessionActionError(false);
      clearResumeFailure();
      void (checked ? start() : stop()).catch(() => {
        if (!sessionActionCoordinator.commandRejected(pendingAction)) return;
        setSessionActionError(true);
        setSessionPendingAction(null);
      });
    },
    [sessionActionCoordinator, start, stop, clearResumeFailure],
  );

  const resumeSession = useCallback(() => {
    const session = useStore.getState().session;
    if (resumeInFlight.current || sessionActionCoordinator.pendingAction !== null ||
      !session.isActive || !session.isPaused || session.status.kind === "connecting" || session.status.kind === "stopping") return;
    resumeInFlight.current = true;
    setSessionActionError(false);
    // Resuming changes isPaused while statusKind can remain "listening".
    // Release this action on IPC completion instead of waiting for a lifecycle
    // transition required by the independent start/stop coordinator.
    void runResume(async () => {
      try {
        await togglePaused();
      } catch (error) {
        const current = useStore.getState().session;
        // A tray/shortcut resume or stop supersedes a late IPC rejection.
        if (current.isActive && current.isPaused) throw error;
      }
    }, I18N.settings.sessionResumeFailed).finally(() => { resumeInFlight.current = false; });
  }, [sessionActionCoordinator, runResume, togglePaused]);

  useEffect(() => {
    return useStore.subscribe((state, previousState) => {
      const statusKind = selectSessionStatusKind(state);
      const previousStatusKind = selectSessionStatusKind(previousState);
      const isActive = state.session.isActive;
      const lifecycleChanged = statusKind !== previousStatusKind || isActive !== previousState.session.isActive;
      if (!lifecycleChanged && state.session.isPaused === previousState.session.isPaused) {
        return;
      }
      // A fresh native transition is authoritative even when it came from the
      // tray or global shortcut. Do not leave an earlier settings IPC failure
      // visible beside a subsequently successful session state.
      setSessionActionError(false);
      clearResumeFailure();
      if (!lifecycleChanged) return;
      const pendingAction = sessionActionCoordinator.observeNativeState({
        statusKind,
        isActive,
      });
      setSessionPendingAction((current) => (current === pendingAction ? current : pendingAction));
    });
  }, [sessionActionCoordinator, clearResumeFailure]);

  return visible ? <SettingsSessionControls
            permissionRequired={permissionRequired}
            compact={compact}
            retrying={sessionPendingAction === "start" && sessionStatusKind === "error"}
            resuming={sessionIsResuming}
            resumeFailed={sessionResumeFailed}
            resumeFailureMessage={sessionResumeFailureMessage}
            checked={sessionControl.checked}
            disabled={sessionControl.disabled || sessionIsResuming}
            status={sessionControl.visibleStatus}
            errorMessage={sessionErrorMessage}
            errorRequiresConfiguration={errorRequiresConfiguration}
            statusText={sessionControl.visibleStatus === "listening" && translationRecoveryReason
              ? translationRecoveryRetryScheduled === false
                ? translationRecoveryReason === "rateLimited" ? I18N.overlay.translationLimited : I18N.overlay.translationUnavailable
                : translationRecoveryReason === "rateLimited" ? I18N.overlay.translationRateLimited : I18N.overlay.translationRetrying
              : settingsSessionStatusText(sessionControl.visibleStatus)}
            isActive={sessionIsActive}
            isChanging={isChangingSession || sessionPendingAction !== null || sessionIsResuming}
            immersive={settings.subtitleBlendsWithBackground}
            canConfigure={sessionControl.canConfigure}
            actionFailed={sessionActionError}
            nativeShortcuts={nativeShortcuts}
            desktopShortcuts={commands}
            onSessionChange={changeSession}
            onResume={resumeSession}
            onImmersiveChange={(subtitleBlendsWithBackground) => void runWithToast(
              () => saveSettings({ subtitleBlendsWithBackground }),
              I18N.settings.settingSaveFailed(I18N.settings.blendBackground),
            )}
            configureLabel={errorSettingsTarget === "appleSpeechResources" ? I18N.settings.appleSpeechOpenResources : undefined}
            onConfigure={() => onConfigure(errorSettingsTarget ?? "service")}
  /> : null;
}

function settingsSessionStatusText(
  status: SettingsSessionVisibleStatus,
): string {
  switch (status) {
    case "idle":
      return I18N.settings.sessionIdle;
    case "connecting":
      return I18N.settings.sessionConnecting;
    case "listening":
      return I18N.settings.sessionListening;
    case "paused":
      return I18N.settings.sessionPaused;
    case "stopping":
      return I18N.settings.sessionStopping;
    case "error":
      return I18N.settings.sessionError;
    case "setupRequired":
      return I18N.settings.sessionSetupRequired;
    case "credentialUnavailable":
      return I18N.settings.sessionCredentialUnavailable;
  }
}
