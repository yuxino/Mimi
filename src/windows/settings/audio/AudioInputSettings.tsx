import { useEffect, useRef, useState } from "react";
import { I18N } from "../../../lib/i18n";
import { useStore } from "../../../lib/store";
import { SettingsRow, SettingsSection } from "../controls/SettingsPrimitives";
import { Switch } from "../../../components/inputs/Switch";
import { audioInputErrorMessage } from "../../../lib/audio/audioInput";
import { applicationAudioError } from "../../../lib/audio/applicationAudio";
import { audioSourceErrorMessage } from "../../../lib/audio/windowsAudioSource";
import { captureSwitchCopy } from "../../../lib/audio/captureStatus";
import type { AudioInput, AudioSource } from "../../../lib/types";
import { ApplicationAudio } from "./ApplicationAudio";
import { WindowsAudioSource } from "./WindowsAudioSource";
import { useSettingsToast } from "../feedback/useSettingsToast";

/** Uses the same idle, live and paused reconfiguration path as the overlay. */
export function AudioInputSettings() {
  const selected = useStore(state => state.settings.audioInput) ?? "system";
  const microphoneAvailable = useStore(state => state.settings.microphoneInputAvailable === true);
  const target = useStore(state => state.settings.systemAudioTarget);
  const status = useStore(state => state.session.status.kind);
  const initialization = useStore(state => state.initializationStatus);
  const switchAudioInput = useStore(state => state.switchAudioInput);
  const [switchingInput, setSwitchingInput] = useState(false);
  const [targetBusy, setTargetBusy] = useState(false);
  const { beginToast } = useSettingsToast();
  const inFlight = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const copy = captureSwitchCopy();
  const sourceControlsDisabled = initialization !== "ready" || status === "connecting" || status === "stopping" || switchingInput || targetBusy;
  const toggle = async (source: AudioSource, checked: boolean) => {
    // One source must remain selected. Enabling the other source means both,
    // never implicitly replaces the user's existing choice.
    if (sourceControlsDisabled || inFlight.current || (!checked && selected === source)) return;
    const value: AudioInput = checked ? "both" : source === "system" ? "microphone" : "system";
    if (value === selected) return;
    inFlight.current = true;
    setSwitchingInput(true);
    const notify = beginToast();
    try { await switchAudioInput(value); }
    catch (error) {
      const message = error instanceof Error ? error.message : typeof error === "string" ? error : "";
      if (mounted.current) notify(applicationAudioError(message) ?? audioInputErrorMessage(message) ?? audioSourceErrorMessage(message) ?? copy.switchFailed, true);
    }
    finally {
      inFlight.current = false;
      if (mounted.current) setSwitchingInput(false);
    }
  };
  return <SettingsSection id="audio-input" title={I18N.settings.audioInputTitle}>
    <SettingsRow label={I18N.settings.audioInputSystem} description={microphoneAvailable ? `${I18N.settings.audioInputHelp}\n${copy.switchHelp}` : undefined}
      hint={microphoneAvailable && selected === "system" ? I18N.settings.audioInputAtLeastOne : undefined}>
      {microphoneAvailable && <Switch aria-label={I18N.settings.audioInputSystem} checked={selected !== "microphone"}
        disabled={sourceControlsDisabled || selected === "system"} onChange={checked => void toggle("system", checked)} />}
    </SettingsRow>
    {microphoneAvailable && <SettingsRow label={I18N.settings.audioInputMicrophone} description={`${I18N.settings.audioInputMicrophoneHelp}\n${copy.switchHelp}`}
      hint={selected === "microphone" ? I18N.settings.audioInputAtLeastOne : undefined}>
      <Switch aria-label={I18N.settings.audioInputMicrophone} checked={selected !== "system"}
        disabled={sourceControlsDisabled || selected === "microphone"} onChange={checked => void toggle("microphone", checked)} />
    </SettingsRow>}
    {/* The picker owns its pending/reconnect state; targetBusy only blocks the
        sibling input switches so it cannot disable its own focused trigger. */}
    {selected !== "microphone" && <ApplicationAudio busy={switchingInput} onBusyChange={setTargetBusy} />}
    {selected !== "microphone" && target?.kind !== "application" && <WindowsAudioSource />}
  </SettingsSection>;
}
