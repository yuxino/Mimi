import { useCallback, useEffect, useRef, useState } from "react";
import { targetLanguagesForSettings } from "../../lib/providerCapabilities";
import {
  isTauri,
  listenOverlayControlMode,
  overlayControlGetState,
  overlayControlSetIslandWidth,
  overlayPopoverHide,
  overlayPopoverToggle,
  type OverlayControlMode,
} from "../../lib/ipc";
import {
  selectHasRecognizingSourceDraft,
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
  const switchSourceLanguage = useStore((state) => state.switchSourceLanguage);
  const switchTargetLanguage = useStore((state) => state.switchTargetLanguage);
  const saveSettings = useStore((state) => state.saveSettings);
  const setOverlayLocked = useStore((state) => state.setOverlayLocked);
  const showSettings = useStore((state) => state.showSettings);
  const [mode, setMode] = useState<OverlayControlMode>(initialPreviewMode);
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
    if (target) await switchTargetLanguage(target);
  };

  const toggle = useCallback(() => {
    if (isTauri) {
      void overlayPopoverToggle().catch(() => {});
    } else {
      setMode((current) => (current === "panel" ? "island" : "panel"));
    }
  }, []);

  const dismiss = useCallback(() => {
    if (isTauri) {
      void overlayPopoverHide().catch(() => {});
    } else {
      setMode("island");
    }
  }, []);

  const reportIslandWidth = useCallback((width: number) => {
    void overlayControlSetIslandWidth(width).catch(() => {});
  }, []);

  useEffect(() => {
    if (!isTauri) return;
    let disposed = false;
    let eventSeen = false;
    let removeListener: (() => void) | undefined;
    void listenOverlayControlMode((nextMode) => {
      eventSeen = true;
      if (!disposed) setMode(nextMode);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      removeListener = unlisten;
      void overlayControlGetState()
        .then((nextMode) => {
          if (!disposed && !eventSeen) setMode(nextMode);
        })
        .catch(() => {});
    });
    return () => {
      disposed = true;
      removeListener?.();
    };
  }, []);

  useEffect(() => {
    if (mode !== "panel") return;
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
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
          onDismiss={dismiss}
          onSwitchSourceLanguage={switchSourceLanguage}
          onSetSkipTranslation={setSkipTranslation}
          onSetTextOpaque={(keepSubtitleTextOpaque) => saveSettings({ keepSubtitleTextOpaque })}
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
