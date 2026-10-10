import { isSystemAudioPermissionDenied } from "../../lib/systemAudioPermissions";
import { useEffect, useMemo, useRef, useState } from "react";
import { subtitleBackgroundColor } from "../../lib/subtitleColor";
import { I18N } from "../../lib/i18n";
import { AudioInputIndicator } from "../../components/AudioInputIndicator";
import { SessionErrorFeedback } from "../../components/SessionErrorFeedback";
import { sessionErrorSettingsTarget } from "../../lib/connectionDiagnostics";
import { audioInputLabel } from "../../lib/audioInput";
import { hasEmbeddedOverlayControls, isTauri, listenOverlayPointerMotion } from "../../lib/ipc";
import { OverlayControlWindow } from "../overlay-control/OverlayControlWindow";
import { useOverlayControlMode } from "../../lib/useOverlayControlMode";
import { selectSessionErrorMessage, selectSessionErrorSummary, useStore } from "../../lib/store";
import { DevelopmentOverlayTrace } from "./DevelopmentOverlayTrace";
import { OVERLAY_ACTIVITY_PHASES, hexToRgba } from "../../lib/types";
import { ControlButton } from "./ControlButton";
import { DragHandle } from "./DragHandle";
import { PulseRing } from "./PulseRing";
import { OverlayLatency } from "./OverlayLatency";
import { OverlayTranslationService } from "./OverlayTranslationService";
import { translationService } from "./translationService";
import { ResizeHandles } from "./ResizeHandles";
import { Timeline } from "./Timeline";
import { useResolvedMotion } from "./animation";
import { useSubtitleTail } from "./useSubtitleTail";
import type { SourceSubtitleSnapshot } from "../../lib/types";
import { overlaySessionChromeLayout } from "./overlayChromeLayout";
import { minimumOverlayHeight } from "./overlayMinimumHeight";
import { useSessionAction } from "./useSessionAction";
import { publishOverlayPointerMotion } from "../../lib/overlayPointer";
import {
  buildSubtitleBlocks,
  buildMultiSourceSubtitleBlocks,
  computeActivityPhase,
  emptyStateDensity,
  emptyStateIsError,
  emptyStateText,
  hasSubtitleContent,
  usesAtomicSubtitlePreview,
  usesStreamingSubtitlePreview,
} from "./overlayModel";

const ACCENT = "#7AA8FF";
const OVERLAY_INSET = 6;
const EMPTY_MICROPHONE: SourceSubtitleSnapshot = {
  audioSource: "microphone", source: { text: "", isFinal: false }, translation: { text: "", isFinal: false },
  history: [], detectedLanguage: null, isTranslationPending: false, isTranslationTimedOut: false,
};
const EMPTY_SYSTEM: SourceSubtitleSnapshot = { ...EMPTY_MICROPHONE, audioSource: "system" };
type ControlAction = "collapse" | "clear" | "immersive" | "lock" | "settings" | "close";

/** Floating subtitle overlay driven by native session and geometry state. */
export function OverlayWindow() {
  const subtitleRootRef = useRef<HTMLDivElement>(null);
  const session = useStore((state) => state.session);
  const sessionErrorMessage = useStore(selectSessionErrorMessage);
  const sessionErrorSummary = useStore(selectSessionErrorSummary);
  const [controlMode] = useOverlayControlMode();
  const hasSessionError = session.status.kind === "error";
  const permissionRequired = useStore(state => state.session.status.kind === "error" && isSystemAudioPermissionDenied(state.session.status.message));
  const errorSettingsTarget = session.status.kind === "error" ? sessionErrorSettingsTarget(session.status.message) : null;
  const errorRequiresConfiguration = errorSettingsTarget !== null;
  const settings = useStore((state) => state.settings);
  const togglePaused = useStore((state) => state.togglePaused);
  const start = useStore((state) => state.start);
  const stop = useStore((state) => state.stop);
  const clearSubtitles = useStore((state) => state.clearSubtitles);
  const setOverlayCollapsed = useStore((state) => state.setOverlayCollapsed);
  const setOverlayLocked = useStore((state) => state.setOverlayLocked);
  const saveSettings = useStore((state) => state.saveSettings);
  const showSettings = useStore((state) => state.showSettings);
  const sessionAction = useSessionAction();
  const controlAction = useSessionAction();
  const actionFailureMessage = sessionAction.failureMessage ?? controlAction.failureMessage ?? I18N.overlay.controlActionFailed;
  const { run: runGuardedControl, clearFailure: clearControlFailure } = controlAction;
  const [pendingControl, setPendingControl] = useState<ControlAction | null>(null);
  const { clearFailure } = sessionAction;
  useEffect(() => {
    clearFailure();
    clearControlFailure();
  }, [session.status.kind, session.isActive, session.isPaused, clearFailure, clearControlFailure]);

  const runControlAction = (action: ControlAction, operation: () => Promise<void>) => {
    void runGuardedControl(async () => {
      setPendingControl(action);
      try {
        await operation();
      } finally {
        setPendingControl(null);
      }
    });
  };

  const [isHovering, setIsHovering] = useState(false);
  const [readingHistory, setReadingHistory] = useState(false);
  const [followTailRequest, setFollowTailRequest] = useState(0);
  useEffect(() => {
    if (!isTauri) return;
    let disposed = false;
    let pointerInside = false;
    let removeListener: (() => void) | undefined;
    const blurWindow = () => {
      pointerInside = false;
      publishOverlayPointerMotion(null);
      setIsHovering(false);
    };
    window.addEventListener("blur", blurWindow);
    void listenOverlayPointerMotion((point) => {
      if (disposed) return;
      publishOverlayPointerMotion(point);
      const nextInside = point !== null;
      if (pointerInside !== nextInside) {
        pointerInside = nextInside;
        setIsHovering(nextInside);
      }
    }).then((unlisten) => {
      if (disposed) unlisten();
      else removeListener = unlisten;
    }).catch(() => {});
    return () => {
      disposed = true;
      window.removeEventListener("blur", blurWindow);
      removeListener?.();
      publishOverlayPointerMotion(null);
    };
  }, []);
  const [overlaySize, setOverlaySize] = useState(() => ({
    width: typeof window === "undefined" || !isTauri ? 640 : window.innerWidth,
    height:
      typeof window === "undefined" || !isTauri ? 482 : window.innerHeight,
  }));
  const topChromeLayout = overlaySessionChromeLayout(
    Math.max(0, overlaySize.width - OVERLAY_INSET * 2),
    session,
  );
  const showSessionControls = topChromeLayout.showControls;
  const service = translationService(settings);
  const separateMetadataRow = Boolean(service) && overlaySize.width < 552;
  const contentTopBandHeight = topChromeLayout.topBandHeight + (separateMetadataRow ? 20 : 0);

  const collapsed = session.isOverlayCollapsed;
  // Native geometry temporarily expands an error surface too. Preserve both
  // presentation preferences so recovery restores the user's reading mode.
  const blendsWithBackground = settings.subtitleBlendsWithBackground && !hasSessionError;
  const presentationLocked = settings.isOverlayLocked && !hasSessionError;
  // Resolved once per render: an explicit switch overrides the system, and the
  // body class is what the CSS animations key off.
  const pulseOn = useResolvedMotion(settings.pulseAnimation);
  const motionOn = useResolvedMotion(settings.subtitleAnimation);
  useEffect(() => {
    document.body.classList.toggle("motion-reduced", !motionOn);
    document.body.classList.toggle("pulse-off", !pulseOn);
  }, [motionOn, pulseOn]);
  const presentationCollapsed = collapsed && !blendsWithBackground && !hasSessionError;
  const phase = computeActivityPhase(session, settings);
  const activeProvider = settings.profiles.find(profile => profile.id === settings.activeProfileId)?.provider;
  const atomicProvider = session.atomicPreview ?? usesAtomicSubtitlePreview(activeProvider);
  const streamingProvider = usesStreamingSubtitlePreview(activeProvider);
  // Disabling an input keeps its confirmed captions and source identity.
  const dual = new Set([
    ...(session.subtitles.tracks ?? []).map(track => track.audioSource),
    ...session.subtitles.history.map(pair => pair.audioSource ?? "system"),
  ]).size > 1;
  const systemTrack = dual ? session.subtitles.tracks?.find(track => track.audioSource === "system") : undefined;
  const microphoneTrack = dual ? session.subtitles.tracks?.find(track => track.audioSource === "microphone") : undefined;
  const primarySubtitles = dual ? systemTrack ?? EMPTY_SYSTEM : session.subtitles;
  const microphoneSubtitles = microphoneTrack ?? EMPTY_MICROPHONE;
  const primaryAudioSource = session.subtitles.tracks?.[0]?.audioSource
    ?? (settings.audioInput === "microphone" ? "microphone" : "system");
  const running = OVERLAY_ACTIVITY_PHASES[phase].animationSpeed > 0;
  const primaryTail = useSubtitleTail(primarySubtitles, settings, dual ? systemTrack ?? EMPTY_SYSTEM : session, running, atomicProvider, "primary", streamingProvider);
  const microphoneTail = useSubtitleTail(microphoneSubtitles, settings, microphoneSubtitles, running, atomicProvider, "microphone", streamingProvider);
  const blocks = useMemo(() => dual
    ? buildMultiSourceSubtitleBlocks(session.subtitles.history, settings.subtitleDisplayMode, [
      { audioSource: "system", history: primarySubtitles.history, tail: primaryTail },
      { audioSource: "microphone", history: microphoneSubtitles.history, tail: microphoneTail },
    ])
    : buildSubtitleBlocks(session.subtitles.history, settings.subtitleDisplayMode, primaryTail)
      .map(block => ({ ...block, audioSource: block.audioSource ?? primaryAudioSource })),
  [dual, session.subtitles.history, settings.subtitleDisplayMode, primarySubtitles.history, primaryTail, primaryAudioSource, microphoneSubtitles.history, microphoneTail]);
  const hasContent = hasSubtitleContent(session.subtitles);
  const phaseLabel = session.status.kind === "stopping" ? I18N.overlay.stopping : OVERLAY_ACTIVITY_PHASES[phase].accessibilityLabel;
  const pauseLabel = session.isPaused
    ? I18N.overlay.resume
    : I18N.overlay.pause;
  const sessionActionBusy = sessionAction.pending || session.status.kind === "connecting" || session.status.kind === "stopping";
  const sessionActionLabel = sessionActionBusy
    ? session.status.kind === "stopping" ? I18N.overlay.stopping : I18N.overlay.connecting
    : errorRequiresConfiguration ? errorSettingsTarget === "appleSpeechResources" ? I18N.settings.appleSpeechOpenResources : I18N.settings.openSpeechSettings
    : hasSessionError ? I18N.overlay.retry : pauseLabel;
  const runSessionAction = () => {
    if (errorRequiresConfiguration) {
      runControlAction("settings", () => showSettings(errorSettingsTarget ?? "service"));
    } else {
      const resuming = !hasSessionError && session.isPaused;
      void sessionAction.run(async () => {
        try {
          await (hasSessionError ? start() : togglePaused());
        } catch (error) {
          const current = useStore.getState().session;
          if (!resuming || (current.isActive && current.isPaused)) throw error;
        }
      });
    }
  };

  const toggleCollapsed = () => {
    runControlAction("collapse", () => setOverlayCollapsed(!collapsed));
  };

  useEffect(() => {
    if (blendsWithBackground && collapsed) {
      void runGuardedControl(() => setOverlayCollapsed(false));
    }
  }, [blendsWithBackground, collapsed, setOverlayCollapsed, runGuardedControl]);

  useEffect(() => {
    if (!isTauri) return;
    const syncViewportSize = () => {
      setOverlaySize({ width: window.innerWidth, height: window.innerHeight });
    };
    syncViewportSize();
    window.addEventListener("resize", syncViewportSize);
    return () => window.removeEventListener("resize", syncViewportSize);
  }, []);

  const handleResize = (width: number, height: number) => {
    setOverlaySize({ width, height });
  };

  const content = (
    <>
      <div ref={subtitleRootRef} className="h-full w-full" style={{ padding: OVERLAY_INSET }}>
        <div
          key={presentationCollapsed ? "collapsed" : "expanded"}
          className={
            presentationCollapsed
              ? "overlay-swap-collapsed"
              : "overlay-swap-expanded"
          }
          style={{ height: "100%", width: "100%" }}
        >
          {presentationCollapsed ? renderCompact() : renderExpanded()}
        </div>
      </div>
      {!settings.isOverlayLocked && !presentationCollapsed && !blendsWithBackground && !hasSessionError && (
        <ResizeHandles disabled={false} onResize={handleResize} minimumHeight={minimumOverlayHeight(settings)} />
      )}
      {__MIMI_DEVELOPMENT_BUILD__ && <DevelopmentOverlayTrace
        subtitleRootRef={subtitleRootRef} session={session} settings={settings} blocks={blocks}
        dual={dual} primarySubtitles={primarySubtitles} microphoneSubtitles={microphoneSubtitles}
        primaryTail={primaryTail} microphoneTail={microphoneTail} atomicProvider={atomicProvider}
        presentationCollapsed={presentationCollapsed}
      />}
    </>
  );

  if (isTauri) {
    return <div className="relative h-full w-full">{content}
      {hasEmbeddedOverlayControls && <OverlayControlWindow embedded />}
    </div>;
  }

  // Plain `vite dev` preview: a fixed box anchored near the bottom-center.
  return (
    <div className="flex h-screen w-screen items-end justify-center pb-[72px]">
      <div
        className="relative"
        style={{ width: overlaySize.width, height: overlaySize.height }}
      >
        {content}
      </div>
    </div>
  );

  function renderStatusLine() {
    const returnToLive = readingHistory && blocks.length > 0 && !presentationCollapsed;
    const showTiming = session.isActive && !blendsWithBackground;
    const actionFailed = sessionAction.failed || controlAction.failed;
    const visibleService = blendsWithBackground ? null : service;
    if (!showTiming && !sessionAction.pending && !actionFailed && !returnToLive && !visibleService) return null;
    return <div className={`overlay-status-row${separateMetadataRow ? " overlay-status-row--narrow" : ""}`}
      style={{ top: contentTopBandHeight - 14, columnGap: separateMetadataRow || blendsWithBackground ? 8 : topChromeLayout.dragHandleWidth + 16 }}>
      <div className="overlay-status-row__leading">
        {sessionAction.pending || actionFailed ? <div role={sessionAction.pending ? "status" : "alert"} className="overlay-action-feedback">
          {sessionAction.pending ? I18N.overlay.connecting : actionFailureMessage}
        </div> : showTiming ? <OverlayLatency session={session} translationRequired={settings.targetLanguage !== "original"} /> : null}
      </div>
      <div className={`overlay-status-row__trailing${returnToLive ? " overlay-status-row__trailing--reading" : ""}`}>
        {visibleService && <OverlayTranslationService service={visibleService}
          onClick={() => runControlAction("settings", () => showSettings("activeProfile"))}
          disabled={controlAction.pending} />}
        {returnToLive && <button type="button" className="overlay-return-to-live" onClick={() => {
          setReadingHistory(false);
          setFollowTailRequest(request => request + 1);
        }}>{I18N.overlay.returnToLive}</button>}
      </div>
    </div>;
  }

  function renderExpanded() {
    const topBandHeight = contentTopBandHeight;
    const emptyDensity = emptyStateDensity(overlaySize.height);
    const compactEmptyPulse = emptyDensity === "compact";
    const pulseBaseSize = compactEmptyPulse ? 48 : 80;
    const emptyFontSize = emptyDensity === "minimal" ? 12 : Math.max(12, settings.fontSize * 0.68);
    const emptyGap = emptyDensity === "comfortable" ? 4 : 2;
    const statusLines = overlaySize.width < 480 ? 2 : 1;
    // Subtract the canvas inset, border, padding and the actual status-line
    // budget so the prominent light also fits short native windows.
    const emptyPulseSize = Math.min(pulseBaseSize, Math.max(0,
      overlaySize.height - 24 - topBandHeight - emptyFontSize * 1.25 * statusLines - emptyGap,
    ));
    const showEmptyPulse = session.isActive && emptyDensity !== "minimal" && emptyPulseSize >= 24;

    if (blendsWithBackground) {
      return (
        <div
          className="relative flex h-full w-full overflow-hidden"
          data-presentation="background-blend"
        >
          {renderStatusLine()}
          <div
            className="flex min-h-0 w-full flex-col"
            style={{
              // Keep the same subtitle content origin as the regular canvas.
              // Only its chrome disappears in Immersive Mode; the text must
              // not jump upward when the presentation changes.
              paddingTop: topBandHeight + 5,
              height: "100%",
            }}
          >
            {blocks.length > 0 && (
              <Timeline
                preserveLiveLineBreaks={streamingProvider}
                blocks={blocks}
                fontSize={settings.fontSize}
              fontFamily={settings.subtitleFontFamily}
                color={settings.subtitleColor}
                alignment={settings.subtitleAlignment}
                displayMode={settings.subtitleDisplayMode}
                showSubtitleDividers={settings.showSubtitleDividers}
                showTimestamps={settings.showSubtitleTimestamps}
                audioInput={settings.audioInput}
                microphoneColor={settings.microphoneSubtitleColor}
                motionEnabled={motionOn}
                blendsWithBackground
                followTailRequest={followTailRequest}
                onReadingHistoryChange={setReadingHistory}
              />
            )}
          </div>
        </div>
      );
    }

    const hoverHighlight = isHovering && !presentationLocked;
    const borderColor = hoverHighlight
      ? hexToRgba(ACCENT, 0.34)
      : "rgba(255,255,255,0.12)";
    const borderWidth = hoverHighlight ? 1 : 0.75;
    return (
      <div
        className="relative h-full w-full overflow-hidden"
        style={{
          borderRadius: 16,
          // Ordinary mode keeps the same canvas through errors and retries.
          // The error feedback owns its contrast; only a saved immersive mode
          // needs a temporary opaque canvas while recovery is interactive.
          background: hasSessionError && settings.subtitleBlendsWithBackground
            ? "rgba(16,16,16,0.96)"
            : subtitleBackgroundColor(settings.subtitleBackgroundOpacity),
          border: `${borderWidth}px solid ${borderColor}`,
        }}
        onMouseEnter={() => setIsHovering(true)}
        onMouseLeave={() => setIsHovering(false)}
      >
        <div className="relative flex h-full flex-col" style={{ padding: 5 }}>
          {renderStatusLine()}
          {/* Top band: the drag handle is absolutely positioned — centered
              horizontally on the window (left 50% + translateX) and pinned
              to the band's bottom — so no flex layout or the capsule/button
              widths can shift it. The band itself only carries the opacity
              fade and lets pointer events through except on the handle. */}
          <div
            className="absolute inset-x-0 top-0"
            style={{
              height: topChromeLayout.topBandHeight,
              pointerEvents: "none",
              // Always-visible drag affordance: dimmed while idle, full on
              // hover. A fully transparent handle leaves no cue that the
              // overlay can be moved.
              opacity: isHovering ? 1 : session.isActive ? 0.45 : 0.6,
              transition: "opacity 160ms ease-out",
            }}
          >
            <div
              style={{
                position: "absolute",
                left: topChromeLayout.dragHandleCenterX,
                bottom: 0,
                transform: "translateX(-50%)",
                pointerEvents: "auto",
              }}
            >
              <DragHandle
                onToggleCollapsed={toggleCollapsed}
                width={topChromeLayout.dragHandleWidth}
                disabled={controlAction.pending}
                collapseDisabled={hasSessionError}
                busy={pendingControl === "collapse"}
              />
            </div>
          </div>

          {showSessionControls &&
            !presentationLocked &&
            topChromeLayout.showPrimaryAction && (
            <div
              className="absolute flex"
              style={{
                top: 10,
                right: 10,
                gap: 4,
                opacity: isHovering || session.isPaused || session.status.kind === "error" ? 1 : 0.75,
                pointerEvents: "auto",
                transition: "opacity 120ms ease",
              }}
            >
              {topChromeLayout.showSessionAction && <ControlButton
                icon={errorRequiresConfiguration ? "gear" : hasSessionError || session.isPaused ? "play" : "pause"}
                label={sessionActionLabel}
                onClick={runSessionAction}
                busy={sessionActionBusy || (errorRequiresConfiguration && controlAction.pending)}
                disabled={sessionActionBusy || controlAction.pending}
              />}
              {topChromeLayout.showActions && <>
              <ControlButton
                icon="chevron-up"
                label={I18N.overlay.collapseSubtitle}
                onClick={() => runControlAction("collapse", () => setOverlayCollapsed(true))}
                busy={pendingControl === "collapse"}
                disabled={controlAction.pending || hasSessionError}
                data-testid="collapse-subtitles"
              />
              <ControlButton
                icon="eraser"
                label={I18N.overlay.clearSubtitles}
                onClick={() => runControlAction("clear", clearSubtitles)}
                busy={pendingControl === "clear"}
                disabled={!hasContent || controlAction.pending}
              />
              <ControlButton
                icon="blend"
                label={I18N.overlay.enterImmersiveMode}
                onClick={() =>
                  runControlAction("immersive", () => saveSettings({ subtitleBlendsWithBackground: true }))
                }
                busy={pendingControl === "immersive"}
                disabled={controlAction.pending || hasSessionError}
                data-testid="toggle-immersive-mode"
              />
              <ControlButton
                icon="lock"
                label={I18N.overlay.lockPosition}
                onClick={() => runControlAction("lock", () => setOverlayLocked(true))}
                busy={pendingControl === "lock"}
                disabled={controlAction.pending || hasSessionError}
                data-testid="toggle-overlay-lock"
              />
              <ControlButton
                icon="gear"
                label={I18N.overlay.openSettings}
                onClick={() => runControlAction("settings", () => showSettings())}
                busy={pendingControl === "settings"}
                disabled={controlAction.pending}
              />
              </>}
              <ControlButton
                icon="close"
                label={I18N.overlay.closeSubtitles}
                onClick={() => runControlAction("close", stop)}
                busy={pendingControl === "close"}
                disabled={controlAction.pending || sessionAction.pending || session.status.kind === "stopping"}
              />
            </div>
          )}

          <div
            className={`flex min-h-0 flex-col${hasSessionError ? " overlay-session-error-content" : ""}`}
            style={{
              // The top band floats over the canvas, so reserve its exact
              // height or subtitle rows will slide underneath the controls.
              // The band includes space below the native control capsule.
              paddingTop: topBandHeight,
              height: "100%",
            }}
          >
          {hasSessionError && <SessionErrorFeedback permissionRequired={permissionRequired}
            message={sessionErrorMessage ?? I18N.settings.sessionError}
            summary={sessionErrorSummary ?? I18N.settings.sessionError}
            actionsHidden={controlMode === "panel"}
            configureLabel={errorSettingsTarget === "appleSpeechResources" ? I18N.settings.appleSpeechOpenResources : undefined}
            onConfigure={() => runControlAction("settings", () => showSettings(errorSettingsTarget ?? "service"))}
            onRetry={errorRequiresConfiguration ? undefined : runSessionAction}
            disabled={sessionActionBusy || controlAction.pending}
          />}
          {blocks.length === 0 ? hasSessionError ? null : (
            <div
              className="flex flex-1 flex-col items-center justify-center"
              style={{ gap: emptyGap }}
            >
              {showEmptyPulse && (
                <div
                  className="flex shrink-0 items-center justify-center"
                  style={{ height: emptyPulseSize, width: emptyPulseSize }}
                >
                  <div style={{ transform: `scale(${emptyPulseSize / pulseBaseSize})` }}>
                    <PulseRing phase={phase} prominent compact={compactEmptyPulse} motionEnabled={pulseOn} pulseStyle={settings.pulseStyle} />
                  </div>
                </div>
              )}
              <div
                style={{
                  width: "100%",
                  minWidth: 0,
                  fontSize: emptyFontSize,
                  lineHeight: 1.25,
                  fontWeight: 500,
                  color: emptyStateIsError(session)
                    ? "rgba(255,69,58,0.9)"
                    : "var(--overlay-empty-text, rgba(255,255,255,0.5))",
                  textAlign: settings.subtitleAlignment,
                  padding: "0 24px",
                  whiteSpace: emptyDensity === "minimal" ? "nowrap" : undefined,
                  overflow: emptyDensity === "minimal" ? "hidden" : undefined,
                  textOverflow:
                    emptyDensity === "minimal" ? "ellipsis" : undefined,
                }}
              >
                {emptyStateText(session, settings)}
              </div>
            </div>
          ) : (
            <Timeline
              preserveLiveLineBreaks={streamingProvider}
              blocks={blocks}
              fontSize={settings.fontSize}
              fontFamily={settings.subtitleFontFamily}
              color={settings.subtitleColor}
              alignment={settings.subtitleAlignment}
              displayMode={settings.subtitleDisplayMode}
              showSubtitleDividers={settings.showSubtitleDividers}
              showTimestamps={settings.showSubtitleTimestamps}
              audioInput={settings.audioInput}
              microphoneColor={settings.microphoneSubtitleColor}
              motionEnabled={motionOn}
              followTailRequest={followTailRequest}
              onReadingHistoryChange={setReadingHistory}
            />
          )}
          </div>
        </div>
      </div>
    );
  }

  function renderCompact() {
    return (
      <div
        className="relative h-full w-full"
        role="group"
        aria-label={`${I18N.overlay.collapsedAccessibilityPrefix}${phaseLabel} · ${audioInputLabel(settings.audioInput, settings.systemAudioTarget)}`}
        style={{
          borderRadius: 14,
          background: subtitleBackgroundColor(settings.subtitleBackgroundOpacity),
          border: `0.75px solid ${hexToRgba(ACCENT, isHovering ? 0.3 : 0.16)}`,
        }}
        onMouseEnter={() => setIsHovering(true)}
        onMouseLeave={() => setIsHovering(false)}
        onWheel={(event) => {
          if (event.deltaY !== 0) {
            event.preventDefault();
            runControlAction("collapse", () => setOverlayCollapsed(false));
          }
        }}
      >
        <div
          className="relative flex h-full items-center"
          style={{ gap: 8, padding: "0 10px" }}
        >
          <DragHandle onToggleCollapsed={toggleCollapsed} compact
            disabled={controlAction.pending} busy={pendingControl === "collapse"} />
          <PulseRing phase={phase} compact motionEnabled={pulseOn} pulseStyle={settings.pulseStyle} />
          <AudioInputIndicator input={settings.audioInput} target={settings.systemAudioTarget} />
          <span
            className="truncate"
            role={sessionAction.failed || controlAction.failed ? "alert" : undefined}
            title={sessionAction.failed || controlAction.failed ? actionFailureMessage : phaseLabel}
            style={{ minWidth: 0, fontSize: 12, fontWeight: 500, color: "rgba(255,255,255,0.76)" }}
          >
            {sessionAction.failed || controlAction.failed ? actionFailureMessage : phaseLabel}
          </span>
          <span className="flex-1" style={{ minWidth: 4 }} />
          <ControlButton
            icon={session.status.kind === "error" || session.isPaused ? "play" : "pause"}
            label={sessionActionLabel}
            onClick={runSessionAction}
            busy={sessionActionBusy}
            disabled={sessionActionBusy || controlAction.pending}
          />
          <ControlButton
            icon="chevron-down"
            label={I18N.overlay.expandSubtitle}
            onClick={() => runControlAction("collapse", () => setOverlayCollapsed(false))}
            busy={pendingControl === "collapse"}
            disabled={controlAction.pending}
            data-testid="expand-subtitles"
          />
          <ControlButton
            icon="close"
            label={I18N.overlay.closeSubtitles}
            onClick={() => runControlAction("close", stop)}
            busy={pendingControl === "close"}
            disabled={controlAction.pending || sessionAction.pending || session.status.kind === "stopping"}
          />
        </div>
      </div>
    );
  }
}
