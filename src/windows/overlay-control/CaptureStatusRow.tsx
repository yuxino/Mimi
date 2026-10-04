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
import { Tooltip } from "../../components/Tooltip";
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
  const microphoneAvailable = useStore(state => state.settings.microphoneInputAvailable === true);
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
  const sources = SOURCES.filter(source => microphoneAvailable || source === "system").map(source => {
    const enabled = input === "both" || input === source;
    const text = capturePresentation(captureStatusForSource(current, input, source), lifecycle, language, source, enabled);
    const help = [text.observation, text.help, microphoneAvailable && input === source ? copy.minimum : null, microphoneAvailable ? copy.switchHelp : null,
      source === "microphone" && input !== "microphone" ? copy.dualHelp : null].filter(Boolean).join("\n");
    return { source, enabled, text, help };
  });
  const groupHelpId = `${id}-help`;
  // The required source is disabled, so keep its status keyboard-accessible
  // through the heading help as well as through its hover tooltip.
  const requiredSource = sources.find(({ source }) => input === source);
  const groupHelp = microphoneAvailable ? [
    requiredSource ? `${requiredSource.text.source} · ${requiredSource.help}` : [copy.minimum, copy.switchHelp].join("\n"),
    copy.dualHelp,
  ].join("\n") : sources[0].help;
  return <div className="overlay-control-capture" aria-label={I18N.settings.audioInputTitle} aria-busy={pending || targetPending}>
    <div className="overlay-control-capture__heading">
      <span>{I18N.settings.audioInputTitle}</span>
      <SettingsHelp id={groupHelpId} text={groupHelp} label={`${I18N.settings.audioInputTitle} · ${I18N.settings.helpLabel}`} />
    </div>
    <div className={`overlay-control-capture__sources${microphoneAvailable ? "" : " is-unavailable"}`} role="group" aria-label={I18N.settings.audioInputTitle}>
      {sources.map(({ source, enabled, text, help }) => {
        const helpId = `${id}-${source}-help`;
        return <div className="overlay-control-capture__source" key={source} data-audio-source={source}>
          {microphoneAvailable && <Tooltip label={help} popupClassName="settings-help-tooltip">
            {() => <button
              type="button"
              role="switch"
              aria-checked={enabled}
              aria-label={text.source}
              aria-describedby={helpId}
              title={text.deviceDescription}
              disabled={locked || input === source}
              data-required={!locked && input === source || undefined}
              className={`overlay-control-capture__toggle${enabled ? " is-on" : ""}`}
              onClick={() => void toggle(source)}
            >
              <Icon name={source === "system" ? "speaker" : "microphone"} />
              <span>{text.source}</span>
              <span className="overlay-control-capture__check" aria-hidden="true">{enabled && <Icon name="checkmark" />}</span>
            </button>}
          </Tooltip>}
          <span id={helpId} className="settings-help-control__description">{help}</span>
        </div>;
      })}
    </div>
    {input !== "microphone" && <span className="overlay-control-capture__application">
      <ApplicationAudioPicker disabled={locked} onBusyChange={setTargetPending} />
    </span>}
    {operationError && <div className="overlay-control-alert" role="alert"><Icon name="exclamation-triangle" /><span>{operationError}</span></div>}
  </div>;
}
