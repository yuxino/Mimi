import { usesWindowsLiveCaptions } from "../../lib/windowsLiveCaptions";
import { useEffect, useRef, useState } from "react";
import { I18N } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { SettingsRow, SettingsSection } from "./SettingsPrimitives";
import { Switch } from "../../components/Switch";
import { audioInputErrorMessage } from "../../lib/audioInput";
import { applicationAudioError } from "../../lib/applicationAudio";
import { audioSourceErrorMessage } from "../../lib/windowsAudioSource";
import { captureSwitchCopy } from "../../lib/captureStatus";
import type { AudioInput, AudioSource } from "../../lib/types";
import { ApplicationAudio } from "./ApplicationAudio";
import { WindowsAudioSource } from "./WindowsAudioSource";
import { useSettingsToast } from "./useSettingsToast";

/** Uses the same idle, live and paused reconfiguration path as the overlay. */
export function AudioInputSettings() {
  const captionReader = useStore(state => usesWindowsLiveCaptions(state.settings));
  const selected = useStore(state => state.settings.audioInput) ?? "system";
  const microphoneAvailable = useStore(state => state.settings.microphoneInputAvailable === true);
  const target = useStore(state => state.settings.systemAudioTarget);
  const status = useStore(state => state.session.status.kind);
  const initialization = useStore(state => state.initializationStatus);
  const switchAudioInput = useStore(state => state.switchAudioInput);
  const [busy, setBusy] = useState(false);
  const [targetBusy, setTargetBusy] = useState(false);
  const { beginToast } = useSettingsToast();
  const inFlight = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const copy = captureSwitchCopy();
  const disabled = status === "connecting" || status === "stopping" || busy || targetBusy || initialization !== "ready";
  const toggle = async (source: AudioSource, checked: boolean) => {
    // One source must remain selected. Enabling the other source means both,
    // never implicitly replaces the user's existing choice.
    if (disabled || inFlight.current || (!checked && selected === source)) return;
    const value: AudioInput = checked ? "both" : source === "system" ? "microphone" : "system";
    if (value === selected) return;
    inFlight.current = true;
    setBusy(true);
    const notify = beginToast();
    try { await switchAudioInput(value); }
    catch (error) {
      const message = error instanceof Error ? error.message : typeof error === "string" ? error : "";
      if (mounted.current) notify(applicationAudioError(message) ?? audioInputErrorMessage(message) ?? audioSourceErrorMessage(message) ?? copy.switchFailed, true);
    }
    finally {
      inFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  if (captionReader) return <SettingsSection id="audio-input" title={I18N.settings.audioInputTitle}><SettingsRow label={I18N.settings.windowsLiveCaptions} description={I18N.settings.windowsLiveCaptionsAudioUnavailable}>{null}</SettingsRow></SettingsSection>;
  return <SettingsSection id="audio-input" title={I18N.settings.audioInputTitle}>
    <SettingsRow label={I18N.settings.audioInputSystem} description={microphoneAvailable ? `${I18N.settings.audioInputHelp}\n${copy.switchHelp}` : I18N.settings.systemAudioHelp}
      hint={microphoneAvailable && selected === "system" ? I18N.settings.audioInputAtLeastOne : undefined}>
      {microphoneAvailable && <Switch aria-label={I18N.settings.audioInputSystem} checked={selected !== "microphone"}
        disabled={disabled || selected === "system"} onChange={checked => void toggle("system", checked)} />}
    </SettingsRow>
    {microphoneAvailable && <SettingsRow label={I18N.settings.audioInputMicrophone} description={`${I18N.settings.audioInputMicrophoneHelp}\n${copy.switchHelp}`}
      hint={selected === "microphone" ? I18N.settings.audioInputAtLeastOne : undefined}>
      <Switch aria-label={I18N.settings.audioInputMicrophone} checked={selected !== "system"}
        disabled={disabled || selected === "microphone"} onChange={checked => void toggle("microphone", checked)} />
    </SettingsRow>}
    {selected !== "microphone" && <ApplicationAudio disabled={disabled} onBusyChange={setTargetBusy} />}
    {selected !== "microphone" && target?.kind !== "application" && <WindowsAudioSource />}
  </SettingsSection>;
}
