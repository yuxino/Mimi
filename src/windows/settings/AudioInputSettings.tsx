import { useEffect, useRef, useState } from "react";
import { I18N } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { InlineFeedback, SettingsRow, SettingsSection } from "./SettingsPrimitives";
import { Switch } from "../../components/Switch";
import { audioInputErrorMessage } from "../../lib/audioInput";
import { captureSwitchCopy } from "../../lib/captureStatus";
import type { AudioInput, AudioSource } from "../../lib/types";
import { WindowsAudioSource } from "./WindowsAudioSource";

/** Uses the same idle, live and paused reconfiguration path as the overlay. */
export function AudioInputSettings() {
  const selected = useStore(state => state.settings.audioInput) ?? "system";
  const status = useStore(state => state.session.status.kind);
  const initialization = useStore(state => state.initializationStatus);
  const switchAudioInput = useStore(state => state.switchAudioInput);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const copy = captureSwitchCopy();
  const disabled = status === "connecting" || status === "stopping" || busy || initialization !== "ready";
  const toggle = async (source: AudioSource, checked: boolean) => {
    // One source must remain selected. Enabling the other source means both,
    // never implicitly replaces the user's existing choice.
    if (disabled || inFlight.current || (!checked && selected === source)) return;
    const value: AudioInput = checked ? "both" : source === "system" ? "microphone" : "system";
    if (value === selected) return;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    try { await switchAudioInput(value); }
    catch (error) {
      const message = error instanceof Error ? error.message : typeof error === "string" ? error : "";
      if (mounted.current) setError(audioInputErrorMessage(message) ?? copy.switchFailed);
    }
    finally {
      inFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  return <SettingsSection id="audio-input" title={I18N.settings.audioInputTitle}>
    <SettingsRow label={I18N.settings.audioInputSystem} description={`${I18N.settings.audioInputHelp}\n${copy.switchHelp}`}
      hint={selected === "system" ? I18N.settings.audioInputAtLeastOne : undefined}>
      <Switch aria-label={I18N.settings.audioInputSystem} checked={selected !== "microphone"}
        disabled={disabled || selected === "system"} onChange={checked => void toggle("system", checked)} />
    </SettingsRow>
    <SettingsRow label={I18N.settings.audioInputMicrophone} description={`${I18N.settings.audioInputMicrophoneHelp}\n${copy.switchHelp}`}
      hint={selected === "microphone" ? I18N.settings.audioInputAtLeastOne : undefined}>
      <Switch aria-label={I18N.settings.audioInputMicrophone} checked={selected !== "system"}
        disabled={disabled || selected === "microphone"} onChange={checked => void toggle("microphone", checked)} />
    </SettingsRow>
    {selected !== "microphone" && <WindowsAudioSource />}
    {error && <InlineFeedback tone="error">{error}</InlineFeedback>}
  </SettingsSection>;
}
