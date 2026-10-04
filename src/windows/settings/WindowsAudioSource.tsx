import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FOLLOW_AUDIBLE,
  FOLLOW_SYSTEM,
  ROLE_COMMUNICATIONS,
  ROLE_MULTIMEDIA,
  audioSourceCopy,
  isDeviceSource,
  type AudioSourceSnapshot,
} from "../../lib/windowsAudioSource";
import { isTauri } from "../../lib/ipc";
import { useStore } from "../../lib/store";
import { SettingsRow, SettingsSelect } from "./SettingsPrimitives";
import { I18N } from "../../lib/i18n";
import { useSettingsToast } from "./useSettingsToast";


export function WindowsAudioSource() {
  const [snapshot, setSnapshot] = useState<AudioSourceSnapshot | null>(null);
  const [failed, setFailed] = useState(false);
  const [saving, setSaving] = useState(false);
  const inFlight = useRef(false);
  const { runWithToast } = useSettingsToast();
  const selected = useStore((state) => state.settings.windowsAudioSource) ?? "";
  const active = useStore((state) => state.session.isActive);
  const paused = useStore((state) => state.session.isPaused);
  const statusKind = useStore((state) => state.session.status?.kind);
  const save = useStore((state) => state.saveSettings);
  const text = audioSourceCopy();
  useEffect(() => {
    if (!isTauri) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      try {
        const value = await invoke<AudioSourceSnapshot | null>("windows_audio_status");
        if (disposed) return;
        setSnapshot(value);
        setFailed(false);
        if (value) timer = setTimeout(() => void refresh(), 1000);
      } catch {
        if (!disposed) {
          setFailed(true);
          if (/Windows/i.test(navigator.userAgent)) {
            setSnapshot((value) => value ?? { devices: [], currentDevice: null, receivingSound: false, receivingAudioData: false });
            timer = setTimeout(() => void refresh(), 1000);
          }
        }
      }
    };
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, []);
  if (!snapshot) return null;
  const missing = isDeviceSource(selected) && !snapshot.devices.some((device) => device.id === selected);
  const current = snapshot.devices.find((device) => device.id === snapshot.currentDevice)?.name;
  const status = failed ? text.failed : missing || snapshot.devices.length === 0 ? text.missing
    : active && !paused ? snapshot.receivingSound ? text.receiving : snapshot.receivingAudioData ? text.silent : text.noData : null;
  const requiresStop = active || paused || statusKind === "connecting" || statusKind === "stopping";
  return (
    <SettingsRow label={text.title} description={requiresStop ? text.stop : `${text.help}\n${text.idle}`}
      feedback={status && <span role="status">{current && active ? `${current} · ` : ""}{status}</span>}>
        <SettingsSelect label={text.title} value={selected} disabled={requiresStop || failed || saving}
          onChange={(value) => {
            if (inFlight.current) return;
            inFlight.current = true;
            setSaving(true);
            void runWithToast(() => save({ windowsAudioSource: value }), I18N.settings.settingSaveFailed(text.title))
              .finally(() => { inFlight.current = false; setSaving(false); });
          }}
          options={[
            { value: FOLLOW_SYSTEM, label: text.system },
            ...(selected === ROLE_COMMUNICATIONS ? [{ value: selected, label: text.communications }] : []),
            ...(selected === ROLE_MULTIMEDIA ? [{ value: selected, label: text.multimedia }] : []),
            ...(selected === FOLLOW_AUDIBLE ? [{ value: selected, label: text.audible }] : []),
            ...(selected === "role:console" ? [{ value: selected, label: text.system }] : []),
            ...snapshot.devices.map((device) => ({ value: device.id, label: device.name })),
            ...(missing ? [{ value: selected, label: text.unavailable }] : []),
          ]} />
    </SettingsRow>
  );
}
