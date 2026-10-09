import { isSystemAudioPermissionDenied } from "../../lib/systemAudioPermissions";
import { useLanguageNormalizationToast } from "../settings/useLanguageNormalizationToast";
import { useCallback, useEffect, useRef } from "react";
import { targetLanguagesForSettings } from "../../lib/providerCapabilities";
import { useOverlayControlMode } from "../../lib/useOverlayControlMode";
import { sessionErrorSettingsTarget } from "../../lib/connectionDiagnostics";
import {
  isTauri,
  overlayControlSetIslandWidth,
  overlayPopoverHide,
  overlayPopoverToggle,
  type OverlayControlMode,
} from "../../lib/ipc";
import {
  selectHasRecognizingSourceDraft,
  selectSessionErrorMessage,
  selectSessionStatusKind,
  useStore,
} from "../../lib/store";
import {
  computeActivityPhaseFromSignals,
  isWaitingForFinalTranslation,
  languageStatus,
  pendingSourceTranslation,
} from "../overlay/overlayModel";
import { LanguageStatusCapsule } from "./LanguageStatusCapsule";
import { OverlayControlPanel } from "./OverlayControlPanel";
import { overlayControlPanelModel } from "./overlayControlModel";
import "./overlay-control.css";

/** Child window that morphs between a compact status island and its panel. */
export function OverlayControlWindow() {
  const sessionStatusKind = useStore(selectSessionStatusKind);
  const sessionErrorMessage = useStore(selectSessionErrorMessage);
  const permissionRequired = useStore(state => state.session.status.kind === "error" && isSystemAudioPermissionDenied(state.session.status.message));
  const errorSettingsTarget = useStore(state => state.session.status.kind === "error" ? sessionErrorSettingsTarget(state.session.status.message) : null);
  const errorRequiresConfiguration = errorSettingsTarget !== null;
  const start = useStore(state => state.start);
  const sessionIsPaused = useStore((state) => state.session.isPaused);
  const detectedLanguage = useStore(
    (state) => state.session.detectedLanguage,
  );
  const isTranslationPending = useStore(
    (state) => state.session.isTranslationPending,
  );
  const sourceTranslationPending = useStore(state => pendingSourceTranslation(state.session.subtitles, state.settings));
  const isTranslationPreviewPending = useStore((state) => state.session.isTranslationPreviewPending);
  const hasRecognizingSourceDraft = useStore(
    selectHasRecognizingSourceDraft,
  );
  const settings = useStore((state) => state.settings);
  const trackLanguageChange = useLanguageNormalizationToast(settings);
  const selectProfile = useStore((state) => state.selectProfile);
  const switchSourceLanguage = useStore((state) => state.switchSourceLanguage);
  const switchTargetLanguage = useStore((state) => state.switchTargetLanguage);
  const saveSettings = useStore((state) => state.saveSettings);
  const setOverlayLocked = useStore((state) => state.setOverlayLocked);
  const showSettings = useStore((state) => state.showSettings);
  const [mode, setMode] = useOverlayControlMode(initialPreviewMode);
  const translationTarget = useRef({ profileId: settings.activeProfileId, language: settings.targetLanguage });
  useEffect(() => {
    if (translationTarget.current.profileId !== settings.activeProfileId || settings.targetLanguage !== "original") {
      translationTarget.current = { profileId: settings.activeProfileId, language: settings.targetLanguage };
    }
  }, [settings.activeProfileId, settings.targetLanguage]);
  const setSkipTranslation = async (enabled: boolean) => {
    const targets = targetLanguagesForSettings(settings);
    const previous = translationTarget.current.profileId === settings.activeProfileId ? translationTarget.current.language : "original";
    const target = enabled ? "original" : previous !== "original" && targets.includes(previous) ? previous : targets.find(language => language !== "original");
    if (target) {
      const finishLanguageChange = trackLanguageChange(target);
      try { await switchTargetLanguage(target); finishLanguageChange(true); }
      catch (error) { finishLanguageChange(false); throw error; }
    }
  };

  const toggle = useCallback(() => {
    if (isTauri) {
      void overlayPopoverToggle().catch(() => {});
    } else {
      setMode((current) => (current === "panel" ? "island" : "panel"));
    }
  }, [setMode]);

  const dismiss = useCallback(() => {
    if (isTauri) {
      void overlayPopoverHide().catch(() => {});
    } else {
      setMode("island");
    }
  }, [setMode]);

  const reportIslandWidth = useCallback((width: number) => {
    void overlayControlSetIslandWidth(width).catch(() => {});
  }, []);

  useEffect(() => {
    if (mode !== "panel") return;
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.isComposing) return;
      event.preventDefault();
      dismiss();
    };
    window.addEventListener("keydown", dismissOnEscape);
    return () => window.removeEventListener("keydown", dismissOnEscape);
  }, [dismiss, mode]);

  const phase = computeActivityPhaseFromSignals(
    {
      statusKind: sessionStatusKind,
      isPaused: sessionIsPaused,
      detectedLanguage,
      isTranslationPending,
      sourceTranslationPending,
      isTranslationPreviewPending,
      hasRecognizingSourceDraft,
    },
    settings,
  );
  const status = languageStatus(settings, settings.audioInput === "both" ? null : detectedLanguage);
  if (status === null) return null;
  const isWaiting = sourceTranslationPending ?? isWaitingForFinalTranslation(
    settings,
    detectedLanguage,
    isTranslationPending,
  );
  const model = overlayControlPanelModel(settings);
  const isChangingSession =
    sessionStatusKind === "connecting" || sessionStatusKind === "stopping";

  return (
    <>
      {mode === "panel" && (
        <OverlayControlPanel
          phase={phase}
          status={status}
          settings={settings}
          model={model}
          isPaused={sessionIsPaused}
          isWaitingForFinalTranslation={isWaiting}
          isChangingSession={isChangingSession}
          isStopping={sessionStatusKind === "stopping"}
          permissionRequired={permissionRequired}
          sessionErrorMessage={sessionErrorMessage}
          errorSettingsTarget={errorSettingsTarget}
          onRetrySession={errorRequiresConfiguration ? undefined : start}
          onDismiss={dismiss}
          onSelectProfile={async (profileId) => { await selectProfile(profileId); }}
          onSwitchSourceLanguage={switchSourceLanguage}
          onSwitchTargetLanguage={async target => {
            const finishLanguageChange = trackLanguageChange(target);
            try { await switchTargetLanguage(target); finishLanguageChange(true); }
            catch (error) { finishLanguageChange(false); throw error; }
          }}
          onSetSkipTranslation={setSkipTranslation}
          onSetIntermediateSubtitles={(showIntermediateSubtitles) => saveSettings({ showIntermediateSubtitles })}
          onSetSubtitleDividers={(showSubtitleDividers) => saveSettings({ showSubtitleDividers })}
          onSetSubtitleTimestamps={(showSubtitleTimestamps) => saveSettings({ showSubtitleTimestamps })}
          onSetSubtitleDisplayMode={(subtitleDisplayMode) => saveSettings({ subtitleDisplayMode })}
          onSetImmersiveMode={(subtitleBlendsWithBackground) =>
            saveSettings({ subtitleBlendsWithBackground })
          }
          onSetOverlayLocked={setOverlayLocked}
          onShowSettings={showSettings}
        />
      )}
      <div
        className={mode === "island" ? undefined : "overlay-control-island-measure"}
        aria-hidden={mode === "island" ? undefined : true}
      >
        <LanguageStatusCapsule
          phase={phase}
          status={status}
          settings={settings}
          isPaused={sessionIsPaused}
          isWaitingForFinalTranslation={isWaiting}
          expanded={false}
          isStopping={sessionStatusKind === "stopping"}
          onToggle={toggle}
          onWidthChange={isTauri ? reportIslandWidth : undefined}
        />
      </div>
    </>
  );
}

function initialPreviewMode(): OverlayControlMode {
  if (isTauri) return "hidden";
  const mode = new URLSearchParams(window.location.search).get("mode");
  return mode === "panel" ? "panel" : "island";
}
