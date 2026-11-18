import { useCallback, useEffect, useRef } from "react";
import { applicationAudioCopy } from "../../../lib/audio/applicationAudio";
import { captureSwitchCopy } from "../../../lib/audio/captureStatus";
import { ApplicationAudioPicker } from "../../../components/audio/ApplicationAudioPicker";
import { SettingsRow } from "../controls/SettingsPrimitives";
import { useSettingsToast } from "../feedback/useSettingsToast";
import { useStore } from "../../../lib/store";

export function ApplicationAudio({ busy = false, onBusyChange }: {
  busy?: boolean;
  onBusyChange?: (busy: boolean) => void;
}) {
  const text = applicationAudioCopy();
  const { beginToast, clearToast } = useSettingsToast();
  const switching = useRef(false);
  const busyChanged = useCallback((busy: boolean) => { switching.current = busy; onBusyChange?.(busy); }, [onBusyChange]);
  const target = useStore(state => state.settings.systemAudioTarget);
  const targetKey = target?.kind === "application" ? target.id : "";
  useEffect(() => { if (!switching.current) clearToast(); }, [targetKey, clearToast]);
  return <SettingsRow label={text.title} description={`${text.help}\n${captureSwitchCopy().switchHelp}`}>
    <ApplicationAudioPicker busy={busy} onBusyChange={busyChanged} onActionStart={() => {
      const notify = beginToast();
      return message => notify(message, true);
    }} />
  </SettingsRow>;
}
