import { applicationAudioCopy } from "../../lib/applicationAudio";
import { captureSwitchCopy } from "../../lib/captureStatus";
import { ApplicationAudioPicker } from "../../components/ApplicationAudioPicker";
import { SettingsRow } from "./SettingsPrimitives";

export function ApplicationAudio({ disabled = false, onBusyChange }: {
  disabled?: boolean;
  onBusyChange?: (busy: boolean) => void;
}) {
  const text = applicationAudioCopy();
  return <SettingsRow label={text.title} description={`${text.help}\n${captureSwitchCopy().switchHelp}`}>
    <ApplicationAudioPicker disabled={disabled} onBusyChange={onBusyChange} />
  </SettingsRow>;
}
