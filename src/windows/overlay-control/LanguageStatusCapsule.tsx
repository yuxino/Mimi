import { useLayoutEffect, useRef } from "react";
import { Icon } from "../../components/Icon";
import { AudioInputIndicator } from "../../components/AudioInputIndicator";
import { RecordingIndicator } from "../../components/RecordingIndicator";
import { audioInputLabel } from "../../lib/audioInput";
import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import { recordingStatusLabel } from "../../lib/recordingStatus";
import { diagnosticPlatform } from "../../lib/connectionDiagnostics";
import {
  OVERLAY_ACTIVITY_PHASES,
  type AudioRecordingStatus,
  type OverlayActivityPhaseKind,
  type SettingsSnapshot,
} from "../../lib/types";
import { PulseRing } from "../overlay/PulseRing";
import { useResolvedMotion } from "../overlay/animation";
import type { LanguageStatus } from "../overlay/overlayModel";
import { capsuleLabels } from "./capsuleLabels";

interface LanguageStatusCapsuleProps {
  phase: OverlayActivityPhaseKind;
  status: LanguageStatus;
  settings: SettingsSnapshot;
  audioRecording?: AudioRecordingStatus;
  isPaused: boolean;
  isWaitingForFinalTranslation: boolean;
  expanded: boolean;
  /** A hidden island still measures its full content without animating it. */
  animationVisible?: boolean;
  isStopping?: boolean;
  onToggle: () => void;
  onWidthChange?: (width: number) => void;
  unlock?: { pending: boolean; failure: string | null; shortcutHint?: string };
}

/** Compact control entry, or a hover-revealed way to unlock the subtitles. */
export function LanguageStatusCapsule({
  phase,
  status,
  settings,
  audioRecording,
  isPaused,
  isWaitingForFinalTranslation,
  expanded,
  animationVisible = true,
  isStopping = false,
  onToggle,
  onWidthChange,
  unlock,
}: LanguageStatusCapsuleProps) {
  const capsuleRef = useRef<HTMLButtonElement>(null);
  const pulseOn = useResolvedMotion(settings.pulseAnimation);
  const transientPhase = phase === "error" || phase === "idle" ? phase
    : isStopping ? "stopping" : phase === "connecting" ? "connecting"
      : isPaused ? "paused" : phase === "translating" || isWaitingForFinalTranslation ? "translating" : null;
  const actionLabel = expanded
    ? I18N.overlay.closeControls
    : unlock ? I18N.overlay.unlockPosition : I18N.overlay.openControls;
  const compact = capsuleLabels(settings, transientPhase);
  const sources = audioInputLabel(settings.audioInput, settings.systemAudioTarget);
  const phaseLabel = isStopping ? I18N.overlay.stopping : OVERLAY_ACTIVITY_PHASES[phase].accessibilityLabel;
  const sourceLabel = compact.source === I18N.settings.recognitionServiceDefault ? compact.source : status.source;
  const recordingLabel = recordingStatusLabel(audioRecording, effectiveUiLanguage());
  const recordingSuffix = recordingLabel ? ` · ${recordingLabel}` : "";
  const fullLabel = `${sources} · ${phaseLabel} · ${sourceLabel} ${status.separator} ${status.target}${recordingSuffix}`;
  const title = unlock
    ? [actionLabel, unlock.shortcutHint, unlock.failure].filter(Boolean).join(" · ")
    : `${fullLabel}. ${actionLabel}`;
  const accessibleLabel = `${sources} · ${phaseLabel}${I18N.overlay.accessibilityCurrentLanguagePrefix}${sourceLabel} ${status.separator} ${status.target}${recordingSuffix}. ${actionLabel}`;

  useLayoutEffect(() => {
    const capsule = capsuleRef.current;
    if (expanded || !onWidthChange || !capsule) return;
    const windowsResize = diagnosticPlatform() === "windows";
    let active = true;
    let lastWidth = 0;
    let lastPixelRatio = 0;
    const measure = () => {
      if (!active) return;
      const width = Math.ceil(capsule.getBoundingClientRect().width);
      const pixelRatio = windowsResize ? window.devicePixelRatio : 1;
      if (width <= 0 || (width === lastWidth && pixelRatio === lastPixelRatio)) return;
      lastWidth = width;
      lastPixelRatio = pixelRatio;
      onWidthChange(width);
    };
    // max-content makes this independent of the previous native window size.
    // Measure synchronously too: a hidden WebView may defer animation frames.
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(capsule);
    if (windowsResize) window.addEventListener("resize", measure);
    return () => {
      active = false;
      observer.disconnect();
      if (windowsResize) window.removeEventListener("resize", measure);
    };
  }, [expanded, onWidthChange]);

  const summary = (
    <>
      <PulseRing phase={phase} compact motionEnabled={pulseOn} animationVisible={animationVisible} pulseStyle={settings.pulseStyle} />
      <AudioInputIndicator input={settings.audioInput} target={settings.systemAudioTarget} />
      {transientPhase && !(transientPhase === "paused" && audioRecording === "paused") && (
        <span className="overlay-control-island__phase">{compact.phase}</span>
      )}
      <span className="overlay-control-island__summary">
        <strong>{compact.source}</strong>
        <span aria-hidden="true">{status.separator}</span>
        <span>{compact.target}</span>
      </span>
      <RecordingIndicator status={audioRecording} language={effectiveUiLanguage()} />
    </>
  );

  if (expanded) {
    return (
      <div className="overlay-control-header">
        <div
          className="overlay-control-header__summary"
          title={fullLabel}
        >
          {summary}
        </div>
        <button
          type="button"
          className="overlay-control-header__collapse"
          onClick={onToggle}
          title={title}
          aria-label={accessibleLabel}
        >
          <Icon name="chevron-up" />
        </button>
      </div>
    );
  }

  return (
    <button
      ref={capsuleRef}
      type="button"
      className="overlay-control-island"
      onClick={onToggle}
      title={title}
      aria-label={unlock ? actionLabel : accessibleLabel}
      aria-haspopup={unlock ? undefined : "dialog"}
      aria-expanded={unlock ? undefined : false}
      aria-controls={unlock ? undefined : "overlay-control-panel"}
      aria-busy={unlock?.pending || undefined}
      disabled={unlock?.pending}
    >
      {unlock ? <>
        <Icon name={unlock.failure ? "exclamation-triangle" : "unlock"} className="overlay-control-island__unlock" />
        <span className="overlay-control-island__summary" role={unlock.failure ? "alert" : undefined}>
          <strong>{unlock.failure ?? actionLabel}</strong>
        </span>
        <RecordingIndicator status={audioRecording} language={effectiveUiLanguage()} />
      </> : <>{summary}<Icon name="chevron-down" /></>}
    </button>
  );
}
