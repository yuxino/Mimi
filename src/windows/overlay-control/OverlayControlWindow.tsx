import { useLanguageNormalizationToast } from "../settings/useLanguageNormalizationToast";
import { useCallback, useEffect, useRef } from "react";
import { targetLanguagesForSettings } from "../../lib/providerCapabilities";
import { useOverlayControlMode } from "../../lib/useOverlayControlMode";
import { sessionErrorSettingsTarget } from "../../lib/connectionDiagnostics";
import {
  isTauri,
  overlayControlSetIslandWidth,
  overlayControlSetPopupBounds,
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
export function OverlayControlWindow({ embedded = false }: { embedded?: boolean }) {
  const surfaceRef = useRef<HTMLDivElement>(null);
  const sessionStatusKind = useStore(selectSessionStatusKind);
  const sessionErrorMessage = useStore(selectSessionErrorMessage);
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

  useEffect(() => {
    if (!embedded || mode !== "panel") return;
    const dismissOutside = (event: PointerEvent) => {
      if (event.target instanceof Element && event.target.closest(".mimi-select__menu, .settings-toast")) return;
      if (event.target instanceof Node && !surfaceRef.current?.contains(event.target)) dismiss();
    };
    window.addEventListener("pointerdown", dismissOutside, true);
    return () => window.removeEventListener("pointerdown", dismissOutside, true);
  }, [dismiss, embedded, mode]);

  useEffect(() => {
    if (!embedded || mode !== "panel") return;
    let frame = 0;
    let previous = "";
    const measure = () => {
      frame = 0;
      const rect = document.querySelector(".mimi-select__menu")?.getBoundingClientRect();
      const bounds = rect ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : null;
      const toastRect = surfaceRef.current?.querySelector(".settings-toast")?.getBoundingClientRect();
      const notification = toastRect ? { x: toastRect.x, y: toastRect.y, width: toastRect.width, height: toastRect.height } : null;
      const key = JSON.stringify([bounds, notification]);
      if (key === previous) return;
      previous = key;
      void overlayControlSetPopupBounds(bounds, notification).catch(() => {});
    };
    const schedule = () => { if (!frame) frame = window.requestAnimationFrame(measure); };
    const observer = new MutationObserver(schedule);
    observer.observe(document.body, { childList: true, characterData: true, subtree: true, attributes: true, attributeFilter: ["style"] });
    window.addEventListener("resize", schedule);
    schedule();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", schedule);
      if (frame) window.cancelAnimationFrame(frame);
      void overlayControlSetPopupBounds(null).catch(() => {});
    };
  }, [embedded, mode]);

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
    <div ref={surfaceRef} className={embedded ? "overlay-control-embedded" : undefined}
      data-mode={embedded ? mode : undefined}>
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
    </div>
  );
}

function initialPreviewMode(): OverlayControlMode {
  if (isTauri) return "hidden";
  const mode = new URLSearchParams(window.location.search).get("mode");
  return mode === "panel" ? "panel" : "island";
}
