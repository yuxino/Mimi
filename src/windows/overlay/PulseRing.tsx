import { memo, useEffect, useState } from "react";
import { OVERLAY_ACTIVITY_PHASES, type OverlayActivityPhaseKind, type PulseStyle } from "../../lib/types";
import "./PulseRing.css";

interface PulseRingProps {
  phase: OverlayActivityPhaseKind;
  compact?: boolean;
  /** A clear focal point while the subtitle overlay is empty. */
  prominent?: boolean;
  /** Already resolved independently from the subtitle-motion preference. */
  motionEnabled: boolean;
  /** Hidden measurement copies keep their geometry and animation phase. */
  animationVisible?: boolean;
  /** Missing preferences use the ribbon (B) indicator. */
  pulseStyle?: PulseStyle;
}

const syllables = [8, 12, 16, 20, 24, 28, 32];
const heights = [6, 12, 23, 29, 22, 13, 6];
// One repeat occupies 40 viewBox units: translating the persistent track
// by 40 units loops without replacing its animation instance.
const wave = "M-40 20 Q-35 20 -30 12 T-20 20 T-10 28 T0 20 T10 12 T20 20 T30 28 T40 20 T50 12 T60 20 T70 28 T80 20";

/** Freeze an outgoing track after its opacity/envelope transition finishes. */
function useTrackClock(active: boolean, enabled: boolean): boolean {
  const [clock, setClock] = useState({ active, enabled, running: active && enabled });
  let running = clock.running;
  if (clock.active !== active || clock.enabled !== enabled) {
    running = enabled && (active || clock.enabled && clock.running);
    setClock({ active, enabled, running });
  }
  useEffect(() => {
    if (!enabled || active || !running) return;
    const settle = window.setTimeout(() => setClock(previous =>
      previous.active || !previous.enabled ? previous : { ...previous, running: false }), 520);
    return () => window.clearTimeout(settle);
  }, [active, enabled, running]);
  return running;
}

/** Decorative sound language from session phases, never measured audio.
 * Tracks stay mounted. Phase changes reshape their enclosing layers;
 * pause settles for 520ms, then freezes clocks; resume continues those clocks.
 */
export const PulseRing = memo(function PulseRing({ phase, compact = false, prominent = false, motionEnabled, animationVisible = true, pulseStyle = "ribbon" }: PulseRingProps) {
  const size = prominent ? (compact ? 48 : 80) : compact ? 24 : 40;
  const working = OVERLAY_ACTIVITY_PHASES[phase].animationSpeed > 0;
  const enabled = motionEnabled && animationVisible;
  const recognizing = working && phase !== "translating";
  const syllableClock = useTrackClock(recognizing && pulseStyle === "syllable", enabled);
  const ribbonClock = useTrackClock(recognizing && pulseStyle === "ribbon", enabled);
  const translationClock = useTrackClock(phase === "translating", enabled);

  return (
    <div
      className={`sound-light${motionEnabled ? "" : " sound-light--still"}`}
      data-phase={phase}
      data-clock={syllableClock || ribbonClock || translationClock ? "running" : "paused"}
      data-sound-style={pulseStyle}
      data-pulse-style={pulseStyle}
      data-size={prominent ? (compact ? "prominent-compact" : "prominent") : compact ? "compact" : "normal"}
      aria-hidden="true"
      style={{ width: size, height: size, color: OVERLAY_ACTIVITY_PHASES[phase].color }}
    >
      <svg className="sound-light__drawing" viewBox="0 0 40 40" fill="none" stroke="currentColor" strokeLinecap="round">
        <g className="sound-light__syllables" data-clock={syllableClock ? "running" : "paused"}>
          {syllables.map((x, index) => (
            <g className="sound-light__envelope" key={x}>
              <g className="sound-light__beat" style={{ animationDelay: `${-index * 0.19}s` }}>
                <path strokeWidth="2.75" d={`M${x} ${20 - heights[index] / 2}V${20 + heights[index] / 2}`} />
              </g>
            </g>
          ))}
        </g>
        <g className="sound-light__ribbons" data-clock={ribbonClock ? "running" : "paused"}>
          <g className="sound-light__ribbon-envelope">
            <path className="sound-light__wave sound-light__wave--back" strokeWidth="1.5" d={wave} />
            <path className="sound-light__wave sound-light__wave--front" strokeWidth="2.2" d={wave} />
          </g>
        </g>
        <g className="sound-light__translation" data-clock={translationClock ? "running" : "paused"}>
          <g className="sound-light__current-envelope">
            <path className="sound-light__current sound-light__current--back" strokeWidth="1.5" d={wave} />
            <path className="sound-light__current sound-light__current--front" strokeWidth="2.2" d={wave} />
          </g>
        </g>
        <path className="sound-light__flat" strokeWidth="2.2" d="M9 20H17M23 20H31" />
        <g className="sound-light__idle" fill="currentColor" stroke="none"><circle cx="16" cy="20" r="1.3" /><circle cx="20" cy="20" r="1.3" /><circle cx="24" cy="20" r="1.3" /></g>
        <path className="sound-light__error" strokeWidth="2.2" d="M16 16L24 24M24 16L16 24" />
      </svg>
    </div>
  );
});
