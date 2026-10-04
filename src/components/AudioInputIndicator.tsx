import { audioInputLabel } from "../lib/audioInput";
import type { AudioInput, SystemAudioTarget } from "../lib/types";
import { Icon } from "./Icon";

/** Selected sources, not a claim that either source is currently receiving sound. */
export function AudioInputIndicator({ input = "system", target }: { input?: AudioInput; target?: SystemAudioTarget }) {
  // System output is the default. Its name remains in the parent control's
  // accessible label; only an explicitly enabled microphone adds chrome.
  if (input === "system") return null;
  const label = audioInputLabel(input, target);
  return <span className="audio-input-indicator" role="img" aria-label={label} title={label}
    style={{ display: "inline-flex", alignItems: "center", gap: 3, flexShrink: 0, fontSize: 14,
      color: "var(--control-text, rgba(255,255,255,0.82))" }}>
    <Icon name="microphone" />
  </span>;
}
