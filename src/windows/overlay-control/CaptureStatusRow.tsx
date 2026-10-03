import { useEffect, useId, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { capturePresentation, captureStatusForSource, captureSwitchCopy, type CaptureStatus } from "../../lib/captureStatus";
import { audioInputErrorMessage } from "../../lib/audioInput";
import { applicationAudioError } from "../../lib/applicationAudio";
import { audioSourceErrorMessage } from "../../lib/windowsAudioSource";
import { isTauri } from "../../lib/ipc";
import { useStore } from "../../lib/store";
import { Icon } from "../../components/Icon";
import { ApplicationAudioPicker } from "../../components/ApplicationAudioPicker";
import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import type { AudioInput, AudioSource } from "../../lib/types";
import { SettingsHelp } from "../settings/SettingsHelp";

const SOURCES: AudioSource[] = ["system", "microphone"];

/** Mounted only in the expanded panel. Status and device details stay in help. */
export function CaptureStatusRow({ disabled = false }: { disabled?: boolean }) {
  const id = useId();
  const [snapshot, setSnapshot] = useState<{ stamp: object; value: CaptureStatus } | null>(null);
  const [pending, setPending] = useState(false);
  const [targetPending, setTargetPending] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const switching = useRef(false);
  const disposed = useRef(false);
  const ready = useStore(state => state.initializationStatus === "ready");
  const active = useStore(state => state.session.isActive);
  const paused = useStore(state => state.session.isPaused);
  const kind = useStore(state => state.session.status.kind);
  const input = useStore(state => state.settings.audioInput) ?? "system";
  const target = useStore(state => state.settings.systemAudioTarget);
  const switchAudioInput = useStore(state => state.switchAudioInput);
  const stamp = useMemo(() => ({ input, target, kind, active, paused }), [input, target, kind, active, paused]);
  useEffect(() => {
    disposed.current = false;
    return () => { disposed.current = true; };
  }, []);
  useEffect(() => {
    if (!isTauri) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      try {
        const value = await invoke<CaptureStatus>("capture_status");
        if (disposed) return;
        setSnapshot({ stamp, value });
      } catch { if (!disposed) setSnapshot(null); }
      if (!disposed) timer = setTimeout(() => void refresh(), 1000);
    };
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [stamp]);
  // Invalidate observations synchronously on input and lifecycle changes.
  const current = snapshot?.stamp === stamp ? snapshot.value : null;
  const language = effectiveUiLanguage();
  const copy = captureSwitchCopy(language);
  const lifecycle = { status: kind === "error" ? { kind, message: "" } : { kind }, isActive: active, isPaused: paused };
  const locked = !ready || disabled || pending || targetPending || kind === "connecting" || kind === "stopping";
  const toggle = async (source: AudioSource) => {
    if (locked || switching.current || input === source) return;
    const next: AudioInput = input === "both" ? source === "system" ? "microphone" : "system" : "both";
    switching.current = true;
    setPending(true);
    setOperationError(null);
    try {
      await switchAudioInput(next);
    } catch (error) {
      const message = error instanceof Error ? error.message : typeof error === "string" ? error : "";
      if (!disposed.current) setOperationError(applicationAudioError(message) ?? audioInputErrorMessage(message) ?? audioSourceErrorMessage(message) ?? copy.switchFailed);
    } finally {
      switching.current = false;
      if (!disposed.current) setPending(false);
    }
  };
  return <div className="overlay-control-capture" aria-label={I18N.settings.audioInputTitle} aria-busy={pending || targetPending}>
    {SOURCES.map(source => {
      const enabled = input === "both" || input === source;
      const text = capturePresentation(captureStatusForSource(current, input, source), lifecycle, language, source, enabled);
      const helpId = `${id}-${source}-help`;
      const switchId = `${id}-${source}-switch`;
      const help = [text.observation, text.help, input === source ? copy.minimum : null, copy.switchHelp, source === "microphone" && input !== "microphone" ? copy.dualHelp : null].filter(Boolean).join("\n");
      return <div className="overlay-control-capture__row" key={source} data-audio-source={source}>
        <label className="overlay-control-capture__source" htmlFor={switchId} title={text.deviceDescription}>
          <span className="overlay-control-setting__icon" aria-hidden="true"><Icon name={source === "system" ? "speaker" : "microphone"} /></span>
          <strong>{text.source}</strong>
        </label>
        <SettingsHelp id={helpId} text={help} label={`${text.source} · ${I18N.settings.helpLabel}`} />
        <button
          id={switchId}
          type="button"
          role="switch"
          aria-checked={enabled}
          aria-label={text.source}
          aria-describedby={helpId}
          disabled={locked || input === source}
          className={`overlay-control-setting overlay-control-capture__toggle${enabled ? " is-on" : ""}`}
          onClick={() => void toggle(source)}
        ><span className="overlay-control-switch" aria-hidden="true"><span /></span></button>
        {source === "system" && enabled && <span className="overlay-control-capture__application">
          <ApplicationAudioPicker disabled={locked} onBusyChange={setTargetPending} />
        </span>}
      </div>;
    })}
    {operationError && <div className="overlay-control-alert" role="alert"><Icon name="exclamation-triangle" /><span>{operationError}</span></div>}
  </div>;
}
