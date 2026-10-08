import { useLayoutEffect, useRef } from "react";
import { Icon } from "../../components/Icon";
import { AudioInputIndicator } from "../../components/AudioInputIndicator";
import { usesWindowsLiveCaptions } from "../../lib/windowsLiveCaptions";
import { audioInputLabel } from "../../lib/audioInput";
import { I18N } from "../../lib/i18n";
import {
  OVERLAY_ACTIVITY_PHASES,
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
  isPaused: boolean;
  isWaitingForFinalTranslation: boolean;
  expanded: boolean;
  isStopping?: boolean;
  onToggle: () => void;
  onWidthChange?: (width: number) => void;
}

/** Compact, always-reachable entry point for the subtitle control panel. */
export function LanguageStatusCapsule({
  phase,
  status,
  settings,
  isPaused,
  isWaitingForFinalTranslation,
  expanded,
  isStopping = false,
  onToggle,
  onWidthChange,
}: LanguageStatusCapsuleProps) {
  const capsuleRef = useRef<HTMLButtonElement>(null);
  const pulseOn = useResolvedMotion(settings.pulseAnimation);
  const transientPhase = phase === "error" || phase === "idle" ? phase
    : isStopping ? "stopping" : phase === "connecting" ? "connecting"
      : isPaused ? "paused" : phase === "translating" || isWaitingForFinalTranslation ? "translating" : null;
  const actionLabel = expanded
    ? I18N.overlay.closeControls
    : I18N.overlay.openControls;
  const compact = capsuleLabels(settings, transientPhase);
  const captionReader = usesWindowsLiveCaptions(settings);
  const sources = captionReader ? I18N.settings.windowsLiveCaptions : audioInputLabel(settings.audioInput, settings.systemAudioTarget);
  const phaseLabel = isStopping ? I18N.overlay.stopping : OVERLAY_ACTIVITY_PHASES[phase].accessibilityLabel;
  const sourceLabel = compact.source === I18N.settings.recognitionServiceDefault ? compact.source : status.source;
  const fullLabel = `${sources} · ${phaseLabel} · ${sourceLabel} ${status.separator} ${status.target}`;

  useLayoutEffect(() => {
    const capsule = capsuleRef.current;
    if (expanded || !onWidthChange || !capsule) return;
    let lastWidth = 0;
    const measure = () => {
      const width = Math.ceil(capsule.getBoundingClientRect().width);
      if (width <= 0 || width === lastWidth) return;
      lastWidth = width;
      onWidthChange(width);
    };
    // max-content makes this independent of the previous native window size.
    // Measure synchronously too: a hidden WebView may defer animation frames.
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(capsule);
    return () => observer.disconnect();
  }, [expanded, onWidthChange]);

  return (
    <button
      ref={capsuleRef}
      type="button"
      className={expanded ? "overlay-control-header" : "overlay-control-island"}
      onClick={onToggle}
      title={`${fullLabel}. ${actionLabel}`}
      aria-label={`${sources} · ${phaseLabel}${I18N.overlay.accessibilityCurrentLanguagePrefix}${sourceLabel} ${status.separator} ${status.target}. ${actionLabel}`}
      aria-haspopup={expanded ? undefined : "dialog"}
      aria-expanded={expanded ? undefined : false}
      aria-controls={expanded ? undefined : "overlay-control-panel"}
    >
      <PulseRing phase={phase} compact motionEnabled={pulseOn} pulseStyle={settings.pulseStyle} />
      {!captionReader && <AudioInputIndicator input={settings.audioInput} target={settings.systemAudioTarget} />}
      {transientPhase && (
        <span className="overlay-control-island__phase">{compact.phase}</span>
      )}
      <span className="overlay-control-island__summary">
        <strong>{compact.source}</strong>
        <span aria-hidden="true">{status.separator}</span>
        <span>{compact.target}</span>
      </span>
      <Icon name={expanded ? "chevron-up" : "chevron-down"} />
    </button>
  );
}
