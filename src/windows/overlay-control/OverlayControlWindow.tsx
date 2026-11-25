import { isSystemAudioPermissionDenied } from "../../lib/audio/systemAudioPermissions";
import { useLanguageNormalizationToast } from "../settings/services/languages/useLanguageNormalizationToast";
import { settingsToastSnapshot, subscribeSettingsToast, useSettingsToast } from "../settings/feedback/useSettingsToast";
import { memo, useCallback, useEffect, useRef, useSyncExternalStore, type RefObject } from "react";
import { effectiveUiLanguage, I18N, subscribeUiLanguage } from "../../lib/i18n";
import { useOverlayControlMode } from "../../lib/overlay/useOverlayControlMode";
import { useDesktopShortcuts } from "../../lib/shortcuts/useDesktopShortcuts";
import { sessionErrorSettingsTarget } from "../../lib/providers/connectionDiagnostics";
import {
  isTauri,
  listenOverlayPointerMotion,
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
  currentTranslationDraftActivity,
  isWaitingForFinalTranslation,
  languageStatus,
  pendingSourceTranslation,
} from "../overlay/timeline/overlayModel";
import { useOverlayChromePeers } from "../overlay/controls/useOverlayChromePeers";
import { useSessionAction } from "../overlay/controls/useSessionAction";
import { LanguageStatusCapsule } from "./capsule/LanguageStatusCapsule";
import { OverlayControlPanel } from "./panel/OverlayControlPanel";
import { overlayControlPanelModel } from "./panel/overlayControlModel";
import "./overlay-control.css";

/** Child window that morphs between a compact status island and its panel. */
export const OverlayControlWindow = memo(function OverlayControlWindow({ embedded = false, subtitleRootRef }: {
  embedded?: boolean;
  subtitleRootRef?: RefObject<HTMLElement | null>;
}) {
  useSyncExternalStore(subscribeUiLanguage, effectiveUiLanguage, effectiveUiLanguage);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const outsidePointerPressPending = useRef(false);
  const sessionStatusKind = useStore(selectSessionStatusKind);
  const sessionErrorMessage = useStore(selectSessionErrorMessage);
  const permissionRequired = useStore(state => state.session.status.kind === "error" && isSystemAudioPermissionDenied(state.session.status.message));
  const errorSettingsTarget = useStore(state => state.session.status.kind === "error" ? sessionErrorSettingsTarget(state.session.status.message) : null);
  const errorRequiresConfiguration = errorSettingsTarget !== null;
  const start = useStore(state => state.start);
  const collapsed = useStore((state) => state.session.isOverlayCollapsed);
  const sessionIsPaused = useStore((state) => state.session.isPaused);
  const audioRecording = useStore((state) => state.session.audioRecording);
  const detectedLanguage = useStore(
    (state) => state.session.detectedLanguage,
  );
  const isTranslationPending = useStore(
    (state) => state.session.isTranslationPending,
  );
  const sourceTranslationPending = useStore(state => pendingSourceTranslation(state.session.subtitles, state.settings));
  const isTranslationPreviewPending = useStore((state) => state.session.isTranslationPreviewPending);
  const hasTranslationDraft = useStore(state => currentTranslationDraftActivity(state.session, state.settings));
  const hasRecognizingSourceDraft = useStore(
    selectHasRecognizingSourceDraft,
  );
  const settings = useStore((state) => state.settings);
  const trackLanguageChange = useLanguageNormalizationToast(settings);
  const selectProfile = useStore((state) => state.selectProfile);
  const switchSourceLanguage = useStore((state) => state.switchSourceLanguage);
  const switchTargetLanguage = useStore((state) => state.switchTargetLanguage);
  const saveSettings = useStore((state) => state.saveSettings);
  const showSettings = useStore((state) => state.showSettings);
  const [mode, setMode] = useOverlayControlMode(initialPreviewMode);
  // Match native recovery, which temporarily lifts the saved lock.
  const lockedPresentation = settings.isOverlayLocked && sessionStatusKind !== "error";
  const visibleMode = lockedPresentation && mode === "panel" ? "island" : mode;
  useEffect(() => {
    // Unlock settings can arrive before the native Island event. Retire the
    // old Panel state so that update cannot reopen controls on its own.
    if (lockedPresentation && mode === "panel") setMode("island");
  }, [lockedPresentation, mode, setMode]);
  const updateNativePointer = useOverlayChromePeers(surfaceRef, "island", isTauri && !embedded);
  useEffect(() => {
    if (!isTauri || embedded) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenOverlayPointerMotion(point => {
      if (!disposed) updateNativePointer(point !== null);
    }, "overlay-control").then(remove => {
      if (disposed) remove();
      else unlisten = remove;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [embedded, updateNativePointer]);
  const quietIsland = (isTauri || embedded) && visibleMode === "island" && sessionStatusKind !== "error" &&
    !collapsed && !settings.subtitleBlendsWithBackground &&
    (!audioRecording || audioRecording === "off");

  const toggle = useCallback(() => {
    if (mode !== "island") return;
    if (isTauri) {
      void overlayPopoverToggle().catch(() => {});
    } else {
      setMode((current) => (current === "panel" ? "island" : "panel"));
    }
  }, [mode, setMode]);

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
    if (mode !== "panel" || lockedPresentation) return;
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
      event.preventDefault();
      dismiss();
    };
    window.addEventListener("keydown", dismissOnEscape);
    return () => window.removeEventListener("keydown", dismissOnEscape);
  }, [dismiss, lockedPresentation, mode]);

  useEffect(() => {
    if (!embedded) return;
    const clearOutsidePress = () => { outsidePointerPressPending.current = false; };
    const consumeOutsideClick = (event: MouseEvent) => {
      // Keyboard and programmatic actions have no preceding pointer press.
      if (!outsidePointerPressPending.current || event.detail === 0) return;
      clearOutsidePress();
      event.preventDefault();
      event.stopImmediatePropagation();
    };
    // This listener outlives Panel → Island so dismissal cannot remove the
    // shield between pointerdown and its re-hit click. A press without a click
    // is retired by the next press, cancellation, blur or unmount.
    window.addEventListener("pointerdown", clearOutsidePress, true);
    window.addEventListener("pointercancel", clearOutsidePress, true);
    window.addEventListener("blur", clearOutsidePress);
    window.addEventListener("click", consumeOutsideClick, true);
    return () => {
      clearOutsidePress();
      window.removeEventListener("pointerdown", clearOutsidePress, true);
      window.removeEventListener("pointercancel", clearOutsidePress, true);
      window.removeEventListener("blur", clearOutsidePress);
      window.removeEventListener("click", consumeOutsideClick, true);
    };
  }, [embedded]);

  useEffect(() => {
    if (!embedded || mode !== "panel" || lockedPresentation) return;
    const dismissOutside = (event: PointerEvent) => {
      if (event.target instanceof Element && event.target.closest(".mimi-select__menu, .settings-toast")) return;
      if (!(event.target instanceof Node) || surfaceRef.current?.contains(event.target)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      outsidePointerPressPending.current = true;
      dismiss();
    };
    window.addEventListener("pointerdown", dismissOutside, true);
    return () => window.removeEventListener("pointerdown", dismissOutside, true);
  }, [dismiss, embedded, lockedPresentation, mode]);

  useEffect(() => {
    if (!embedded || mode !== "panel" || lockedPresentation) return;
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
    const observer = new MutationObserver(records => {
      const subtitleRoot = subtitleRootRef?.current;
      const surface = surfaceRef.current;
      // Only this known fixed-size sibling is independent of the hit slots.
      // Unknown roots, ancestors and portals retain the original policy.
      if (subtitleRoot && surface && !subtitleRoot.contains(surface) && !surface.contains(subtitleRoot)
        && records.every(record => isLocalSubtitleMutation(record, subtitleRoot))) return;
      schedule();
    });
    observer.observe(document.body, { childList: true, characterData: true, subtree: true, attributes: true, attributeFilter: ["style"] });
    window.addEventListener("resize", schedule);
    schedule();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", schedule);
      if (frame) window.cancelAnimationFrame(frame);
      void overlayControlSetPopupBounds(null).catch(() => {});
    };
  }, [embedded, lockedPresentation, mode, subtitleRootRef]);

  const phase = computeActivityPhaseFromSignals(
    {
      statusKind: sessionStatusKind,
      isPaused: sessionIsPaused,
      detectedLanguage,
      isTranslationPending,
      sourceTranslationPending,
      isTranslationPreviewPending,
      hasTranslationDraft,
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
  const capsuleProps = {
    phase,
    status,
    settings,
    audioRecording,
    isPaused: sessionIsPaused,
    isWaitingForFinalTranslation: isWaiting,
    expanded: false,
    animationVisible: mode === "island" && !lockedPresentation,
    isStopping: sessionStatusKind === "stopping",
    onWidthChange: isTauri ? reportIslandWidth : undefined,
  };

  return (
    <div ref={surfaceRef} className={embedded ? "overlay-control-embedded" : "overlay-control-detached"}
      aria-hidden={visibleMode === "hidden" ? true : undefined}
      inert={visibleMode === "hidden" ? true : undefined}
      data-mode={embedded ? visibleMode : undefined} data-quiet-island={quietIsland ? "" : undefined}>
      {mode === "panel" && !lockedPresentation && (
        <OverlayControlPanel
          phase={phase}
          status={status}
          settings={settings}
          audioRecording={audioRecording}
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
          onSetSubtitleDisplayMode={(subtitleDisplayMode) => saveSettings({ subtitleDisplayMode })}
          onShowSettings={showSettings}
        />
      )}
      <div
        className={visibleMode === "island" ? undefined : "overlay-control-island-measure"}
        aria-hidden={visibleMode === "island" ? undefined : true}
      >
        {lockedPresentation
          ? <LockedLanguageStatusCapsule {...capsuleProps} active={visibleMode === "island"} />
          : <LanguageStatusCapsule {...capsuleProps} onToggle={toggle} />}
      </div>
    </div>
  );
});

/** Shortcut discovery and unlock feedback update only the locked capsule. */
function LockedLanguageStatusCapsule({ active, ...props }: Omit<Parameters<typeof LanguageStatusCapsule>[0], "onToggle" | "unlock"> & { active: boolean }) {
  const setOverlayLocked = useStore(state => state.setOverlayLocked);
  const { nativeShortcuts } = useDesktopShortcuts();
  const { pending, run } = useSessionAction();
  const { runWithToast, clearToast } = useSettingsToast();
  const toast = useSyncExternalStore(subscribeSettingsToast, settingsToastSnapshot);
  useEffect(() => {
    if (!active) return;
    const clearWhenHidden = () => { if (document.hidden) clearToast(); };
    window.addEventListener("blur", clearToast);
    document.addEventListener("visibilitychange", clearWhenHidden);
    return () => {
      window.removeEventListener("blur", clearToast);
      document.removeEventListener("visibilitychange", clearWhenHidden);
      clearToast();
    };
  }, [active, clearToast]);
  return <LanguageStatusCapsule {...props}
    unlock={{
      pending,
      failure: active && toast?.failure ? toast.message : null,
      shortcutHint: nativeShortcuts ? `${/Mac|iPhone|iPad|iPod/i.test(navigator.userAgent) ? "⌘⇧" : "Ctrl+Shift+"}L` : undefined,
    }}
    onToggle={() => {
      if (!active) return;
      void run(async () => { await runWithToast(() => setOverlayLocked(false), I18N.overlay.controlActionFailed); });
    }}
  />;
}

// The body observer supplies only child-list, text, and style-attribute changes.
function isLocalSubtitleMutation(record: MutationRecord, subtitleRoot: HTMLElement): boolean {
  if (!subtitleRoot.contains(record.target)) return false;
  // Stylesheets can affect siblings. Never discard explicit layer records.
  const external = "style, link, .mimi-select__menu, .settings-toast";
  const target = record.target instanceof Element ? record.target : record.target.parentElement;
  if (target?.closest(external)) return false;
  return ![...record.addedNodes, ...record.removedNodes].some(node =>
    node instanceof Element && (node.matches(external) || node.querySelector(external)),
  );
}

function initialPreviewMode(): OverlayControlMode {
  if (isTauri) return "hidden";
  const mode = new URLSearchParams(window.location.search).get("mode");
  return mode === "panel" ? "panel" : "island";
}
