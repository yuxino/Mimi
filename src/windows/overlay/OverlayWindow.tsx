import { useEffect, useMemo, useState } from "react";
import { subtitleBackgroundColor } from "../../lib/subtitleColor";
import { I18N } from "../../lib/i18n";
import { AudioInputIndicator } from "../../components/AudioInputIndicator";
import { audioInputLabel } from "../../lib/audioInput";
import { isTauri, listenOverlayPointerMotion } from "../../lib/ipc";
import { useStore } from "../../lib/store";
import { OVERLAY_ACTIVITY_PHASES, hexToRgba } from "../../lib/types";
import { ControlButton } from "./ControlButton";
import { DragHandle } from "./DragHandle";
import { PulseRing } from "./PulseRing";
import { OverlayLatency } from "./OverlayLatency";
import { ResizeHandles } from "./ResizeHandles";
import { Timeline } from "./Timeline";
import { useResolvedMotion } from "./animation";
import { useSubtitleTail } from "./useSubtitleTail";
import type { SourceSubtitleSnapshot } from "../../lib/types";
import { overlaySessionChromeLayout } from "./overlayChromeLayout";
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
} from "./overlayModel";

const ACCENT = "#7AA8FF";
const OVERLAY_INSET = 6;
const EMPTY_MICROPHONE: SourceSubtitleSnapshot = {
  audioSource: "microphone", source: { text: "", isFinal: false }, translation: { text: "", isFinal: false },
  history: [], detectedLanguage: null, isTranslationPending: false, isTranslationTimedOut: false,
};
const EMPTY_SYSTEM: SourceSubtitleSnapshot = { ...EMPTY_MICROPHONE, audioSource: "system" };
type ControlAction = "collapse" | "clear" | "immersive" | "lock" | "settings";

/** Floating subtitle overlay driven by native session and geometry state. */
export function OverlayWindow() {
  const session = useStore((state) => state.session);
  const settings = useStore((state) => state.settings);
  const togglePaused = useStore((state) => state.togglePaused);
  const start = useStore((state) => state.start);
  const clearSubtitles = useStore((state) => state.clearSubtitles);
  const setOverlayCollapsed = useStore((state) => state.setOverlayCollapsed);
  const setOverlayLocked = useStore((state) => state.setOverlayLocked);
  const saveSettings = useStore((state) => state.saveSettings);
  const showSettings = useStore((state) => state.showSettings);
  const sessionAction = useSessionAction();
  const controlAction = useSessionAction();
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
      typeof window === "undefined" || !isTauri ? 136 : window.innerHeight,
  }));
  const topChromeLayout = overlaySessionChromeLayout(
    Math.max(0, overlaySize.width - OVERLAY_INSET * 2),
    session,
  );
  const showSessionControls = topChromeLayout.showControls;

  const collapsed = session.isOverlayCollapsed;
  const blendsWithBackground = settings.subtitleBlendsWithBackground;
  // Resolved once per render: an explicit switch overrides the system, and the
  // body class is what the CSS animations key off.
  const pulseOn = useResolvedMotion(settings.pulseAnimation);
  const motionOn = useResolvedMotion(settings.subtitleAnimation);
  useEffect(() => {
    document.body.classList.toggle("motion-reduced", !motionOn);
    document.body.classList.toggle("pulse-off", !pulseOn);
  }, [motionOn, pulseOn]);
  const presentationCollapsed = collapsed && !blendsWithBackground;
  const phase = computeActivityPhase(session, settings);
  const activeProvider = settings.profiles.find(profile => profile.id === settings.activeProfileId)?.provider;
  const atomicProvider = activeProvider === "alibabaCloud" || activeProvider === "deepLX";
  // Disabling an input keeps its confirmed captions and source identity.
  const dual = new Set([
    ...(session.subtitles.tracks ?? []).map(track => track.audioSource),
    ...session.subtitles.history.map(pair => pair.audioSource ?? "system"),
  ]).size > 1;
  const systemTrack = dual ? session.subtitles.tracks?.find(track => track.audioSource === "system") : undefined;
  const microphoneTrack = dual ? session.subtitles.tracks?.find(track => track.audioSource === "microphone") : undefined;
  const primarySubtitles = dual ? systemTrack ?? EMPTY_SYSTEM : session.subtitles;
  const microphoneSubtitles = microphoneTrack ?? EMPTY_MICROPHONE;
  const running = OVERLAY_ACTIVITY_PHASES[phase].animationSpeed > 0;
  const primaryTail = useSubtitleTail(primarySubtitles, settings, dual ? systemTrack ?? EMPTY_SYSTEM : session, running, atomicProvider, "primary");
  const microphoneTail = useSubtitleTail(microphoneSubtitles, settings, microphoneSubtitles, running, atomicProvider, "microphone");
  const blocks = useMemo(() => dual
    ? buildMultiSourceSubtitleBlocks(session.subtitles.history, settings.subtitleDisplayMode, [
      { audioSource: "system", history: primarySubtitles.history, tail: primaryTail },
      { audioSource: "microphone", history: microphoneSubtitles.history, tail: microphoneTail },
    ])
    : buildSubtitleBlocks(session.subtitles.history, settings.subtitleDisplayMode, primaryTail)
      .map(block => ({ ...block, audioSource: undefined })),
  [dual, session.subtitles.history, settings.subtitleDisplayMode, primarySubtitles.history, primaryTail, microphoneSubtitles.history, microphoneTail]);
  const hasContent = hasSubtitleContent(session.subtitles);

  const phaseLabel = session.status.kind === "stopping" ? I18N.overlay.stopping : OVERLAY_ACTIVITY_PHASES[phase].accessibilityLabel;
  const pauseLabel = session.isPaused
    ? I18N.overlay.resume
    : I18N.overlay.pause;
  const sessionActionBusy = sessionAction.pending || session.status.kind === "connecting" || session.status.kind === "stopping";
  const sessionActionLabel = sessionActionBusy
    ? session.status.kind === "stopping" ? I18N.overlay.stopping : I18N.overlay.connecting
    : session.status.kind === "error" ? I18N.overlay.retry : pauseLabel;
  const runSessionAction = () => {
    void sessionAction.run(session.status.kind === "error" ? start : togglePaused);
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
      <div className="h-full w-full" style={{ padding: OVERLAY_INSET }}>
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
      {!settings.isOverlayLocked && !presentationCollapsed && !blendsWithBackground && (
        <ResizeHandles disabled={false} onResize={handleResize} />
      )}
    </>
  );

  if (isTauri) {
    return <div className="relative h-full w-full">{content}</div>;
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
    if (!showTiming && !sessionAction.pending && !actionFailed && !returnToLive) return null;
    return <div className="overlay-status-row" style={{ top: topChromeLayout.topBandHeight - 14 }}>
      {sessionAction.pending || actionFailed ? <div role={sessionAction.pending ? "status" : "alert"} className="overlay-action-feedback">
        {sessionAction.pending ? I18N.overlay.connecting : I18N.overlay.controlActionFailed}
      </div> : showTiming ? <OverlayLatency session={session} translationRequired={settings.targetLanguage !== "original"} /> : null}
      {returnToLive && <button type="button" className="overlay-return-to-live" onClick={() => {
        setReadingHistory(false);
        setFollowTailRequest(request => request + 1);
      }}>{I18N.overlay.returnToLive}</button>}
    </div>;
  }

  function renderExpanded() {
    const topBandHeight = topChromeLayout.topBandHeight;
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
                blocks={blocks}
                fontSize={settings.fontSize}
                color={settings.subtitleColor}
                alignment={settings.subtitleAlignment}
                displayMode={settings.subtitleDisplayMode}
                showSubtitleDividers={settings.showSubtitleDividers}
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

    const hoverHighlight = isHovering && !settings.isOverlayLocked;
    const borderColor = hoverHighlight
      ? hexToRgba(ACCENT, 0.34)
      : "rgba(255,255,255,0.12)";
    const borderWidth = hoverHighlight ? 1 : 0.75;
    return (
      <div
        className="relative h-full w-full overflow-hidden"
        style={{
          borderRadius: 16,
          background: subtitleBackgroundColor(settings.subtitleBackgroundOpacity),
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
              height: topBandHeight,
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
              />
            </div>
          </div>

          {showSessionControls &&
            !settings.isOverlayLocked &&
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
              <ControlButton
                icon={session.status.kind === "error" || session.isPaused ? "play" : "pause"}
                label={sessionActionLabel}
                onClick={runSessionAction}
                busy={sessionActionBusy}
                disabled={sessionActionBusy}
              />
              {topChromeLayout.showActions && <>
              <ControlButton
                icon="chevron-up"
                label={I18N.overlay.collapseSubtitle}
                onClick={() => runControlAction("collapse", () => setOverlayCollapsed(true))}
                busy={pendingControl === "collapse"}
                disabled={controlAction.pending}
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
                disabled={controlAction.pending}
                data-testid="toggle-immersive-mode"
              />
              <ControlButton
                icon="lock"
                label={I18N.overlay.lockPosition}
                onClick={() => runControlAction("lock", () => setOverlayLocked(true))}
                busy={pendingControl === "lock"}
                disabled={controlAction.pending}
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
            </div>
          )}

          <div
            className="flex min-h-0 flex-col"
            style={{
              // The top band floats over the canvas, so reserve its exact
              // height or subtitle rows will slide underneath the controls.
              // The band includes space below the native control capsule.
              paddingTop: topBandHeight,
              height: "100%",
            }}
          >
          {blocks.length === 0 ? (
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
              blocks={blocks}
              fontSize={settings.fontSize}
              color={settings.subtitleColor}
              alignment={settings.subtitleAlignment}
              displayMode={settings.subtitleDisplayMode}
              showSubtitleDividers={settings.showSubtitleDividers}
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
          <DragHandle onToggleCollapsed={toggleCollapsed} compact />
          <PulseRing phase={phase} compact motionEnabled={pulseOn} pulseStyle={settings.pulseStyle} />
          <AudioInputIndicator input={settings.audioInput} target={settings.systemAudioTarget} />
          <span
            className="truncate"
            role={sessionAction.failed || controlAction.failed ? "alert" : undefined}
            title={sessionAction.failed || controlAction.failed ? I18N.overlay.controlActionFailed : phaseLabel}
            style={{ minWidth: 0, fontSize: 12, fontWeight: 500, color: "rgba(255,255,255,0.76)" }}
          >
            {sessionAction.failed || controlAction.failed ? I18N.overlay.controlActionFailed : phaseLabel}
          </span>
          <span className="flex-1" style={{ minWidth: 4 }} />
          <ControlButton
            icon={session.status.kind === "error" || session.isPaused ? "play" : "pause"}
            label={sessionActionLabel}
            onClick={runSessionAction}
            busy={sessionActionBusy}
            disabled={sessionActionBusy}
          />
          <ControlButton
            icon="chevron-down"
            label={I18N.overlay.expandSubtitle}
            onClick={() => runControlAction("collapse", () => setOverlayCollapsed(false))}
            busy={pendingControl === "collapse"}
            disabled={controlAction.pending}
            data-testid="expand-subtitles"
          />
        </div>
      </div>
    );
  }
}
