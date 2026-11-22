import { applicationAudioCopy } from "../../../lib/audio/applicationAudio";
import { captureSwitchCopy } from "../../../lib/audio/captureStatus";
import { ApplicationAudioPicker } from "../../../components/audio/ApplicationAudioPicker";
import { SettingsRow } from "../controls/SettingsPrimitives";

export function ApplicationAudio({ busy = false, onBusyChange }: {
  busy?: boolean;
  onBusyChange?: (busy: boolean) => void;
}) {
  const text = applicationAudioCopy();
  return <SettingsRow label={text.title} description={`${text.help}\n${captureSwitchCopy().switchHelp}`}>
    <ApplicationAudioPicker busy={busy} onBusyChange={onBusyChange} />
  </SettingsRow>;
}
