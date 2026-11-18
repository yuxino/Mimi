import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "../../../components/Icon";
import "./session-audio-player.css";

const PLAY_START_TIMEOUT_MS = 10_000;

export interface SessionAudioPlayerLabels {
  play: string;
  pause: string;
  loading: string;
  retry: string;
  seek: string;
  playFailed: string;
  ready: string;
  label: string;
}

export interface SessionAudioPlayerProps {
  src: string | null;
  loading: boolean;
  readError: boolean;
  disabled: boolean;
  onLoad: () => void;
  sourceControl?: ReactNode;
  labels: SessionAudioPlayerLabels;
}

function timeLabel(value: number) {
  const seconds = Math.floor(Math.max(0, value));
  return `${Math.floor(seconds / 60).toString().padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
}

/** The parent owns the bounded file read and object URL. Key this component by
 * session/source so an unloaded play request cannot transfer to another track. */
export function SessionAudioPlayer({ src, loading, readError, disabled, onLoad, sourceControl, labels }: SessionAudioPlayerProps) {
  const audio = useRef<HTMLAudioElement | null>(null);
  const requested = useRef(false);
  const attempt = useRef(0);
  const pending = useRef(false);
  const playAllowed = useRef(false);
  const playTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const previousSrc = useRef(src);
  const [playing, setPlaying] = useState(false);
  const [starting, setStarting] = useState(false);
  const [playError, setPlayError] = useState(false);
  const [duration, setDuration] = useState<number | null>(null);
  const [position, setPosition] = useState(0);
  const [renderedOwner, setRenderedOwner] = useState({ src, disabled, readError });
  if (renderedOwner.src !== src || renderedOwner.disabled !== disabled || renderedOwner.readError !== readError) {
    setRenderedOwner({ src, disabled, readError });
    if (renderedOwner.src !== src) {
      setPlaying(false);
      setStarting(false);
      setPlayError(false);
      setPosition(0);
      setDuration(null);
    }
    if (disabled || readError) {
      setPlaying(false);
      setStarting(false);
    }
  }

  const clearPlayTimer = useCallback(() => {
    if (playTimer.current !== null) clearTimeout(playTimer.current);
    playTimer.current = null;
  }, []);

  const attachAudio = useCallback((element: HTMLAudioElement | null) => {
    const previous = audio.current;
    if (previous && previous !== element) {
      attempt.current += 1;
      pending.current = false;
      playAllowed.current = false;
      clearPlayTimer();
      // Releasing a Blob URL or removing the node does not stop its buffer.
      previous.pause();
      previous.removeAttribute("src");
      previous.load();
    }
    audio.current = element;
    // StrictMode can detach and reattach this same DOM node without writing its
    // props again. Restore the source removed by the symmetric release above.
    if (element && src && element.getAttribute("src") !== src) element.setAttribute("src", src);
  }, [clearPlayTimer, src]);

  useEffect(() => {
    if (previousSrc.current === src) return;
    // Preserve the first deliberate request while its file arrives. Replacing
    // an already loaded source always clears that intent.
    if (previousSrc.current !== null || src === null) requested.current = false;
    previousSrc.current = src;
  }, [src]);

  useEffect(() => {
    if (!disabled && !readError) return;
    requested.current = false;
    attempt.current += 1;
    pending.current = false;
    playAllowed.current = false;
    clearPlayTimer();
    audio.current?.pause();
  }, [clearPlayTimer, disabled, readError]);

  function updateTime(element: HTMLAudioElement) {
    if (element !== audio.current) return;
    const length = Number.isFinite(element.duration) && element.duration > 0 ? element.duration : null;
    const current = Number.isFinite(element.currentTime) ? Math.max(0, element.currentTime) : 0;
    setDuration(length);
    setPosition(length === null ? 0 : Math.min(current, length));
  }

  const start = useCallback((element: HTMLAudioElement) => {
    if (disabled || !src || element !== audio.current || !requested.current || pending.current) return;
    requested.current = false;
    pending.current = true;
    playAllowed.current = true;
    const currentAttempt = ++attempt.current;
    setStarting(true);
    setPlayError(false);
    const current = () => currentAttempt === attempt.current && element === audio.current;
    const failed = () => {
      if (!current()) return;
      clearPlayTimer();
      attempt.current += 1;
      pending.current = false;
      playAllowed.current = false;
      setStarting(false);
      setPlaying(false);
      setPlayError(true);
      element.pause();
    };
    // A native media promise can remain pending without an error event. Return
    // control to the same retry action and reject any late playback afterward.
    playTimer.current = setTimeout(failed, PLAY_START_TIMEOUT_MS);
    try {
      // A browser may reject an attempt made after the IPC read. Keep the same
      // button retryable; never claim playback from the click alone.
      void Promise.resolve(element.play()).then(() => {
        if (!current()) return;
        clearPlayTimer();
        pending.current = false;
        setStarting(false);
      }, failed);
    } catch { failed(); }
  }, [clearPlayTimer, disabled, src]);

  useEffect(() => {
    // WKWebView may defer metadata until play() initiates loading. The user's
    // request survives the IPC read, so do not make playback wait for preload.
    // Let this commit finish its StrictMode ref replay before consuming it.
    const element = audio.current;
    if (element) queueMicrotask(() => start(element));
  }, [start]);

  function togglePlayback() {
    if (disabled || loading || pending.current) return;
    const element = audio.current;
    if (playing && element) {
      requested.current = false;
      attempt.current += 1;
      playAllowed.current = false;
      clearPlayTimer();
      element.pause();
      return;
    }
    if (!src) {
      if (requested.current) return;
      requested.current = true;
      setPlayError(false);
      try { onLoad(); } catch {
        requested.current = false;
        setPlayError(true);
      }
      return;
    }
    if (!element) return;
    requested.current = true;
    setPlayError(false);
    if (element.ended || (duration !== null && element.currentTime >= duration)) element.currentTime = 0;
    if (element.error) element.load();
    start(element);
  }

  const failed = readError || playError;
  const busy = loading || starting;
  const buttonLabel = busy ? labels.loading : failed ? labels.retry : playing ? labels.pause : labels.play;
  const time = `${timeLabel(position)} / ${duration === null ? "—:—" : timeLabel(duration)}`;

  return (
    <div className="session-audio-player" role="group" aria-label={labels.label}>
      <audio key={src ?? "unloaded"} hidden ref={attachAudio} src={src ?? undefined} preload="metadata"
        onLoadedMetadata={event => { updateTime(event.currentTarget); start(event.currentTarget); }}
        onDurationChange={event => updateTime(event.currentTarget)}
        onTimeUpdate={event => updateTime(event.currentTarget)}
        onPlaying={event => {
          if (event.currentTarget !== audio.current) return;
          if (disabled || !playAllowed.current) { event.currentTarget.pause(); return; }
          clearPlayTimer();
          attempt.current += 1;
          pending.current = false;
          setStarting(false);
          setPlaying(true);
          setPlayError(false);
        }}
        onPause={event => {
          if (event.currentTarget !== audio.current) return;
          clearPlayTimer();
          requested.current = false;
          attempt.current += 1;
          pending.current = false;
          playAllowed.current = false;
          setStarting(false);
          setPlaying(false);
        }}
        onEnded={event => {
          if (event.currentTarget !== audio.current) return;
          requested.current = false;
          attempt.current += 1;
          pending.current = false;
          playAllowed.current = false;
          clearPlayTimer();
          setPlaying(false);
          setStarting(false);
          updateTime(event.currentTarget);
        }}
        onError={event => {
          if (event.currentTarget !== audio.current) return;
          requested.current = false;
          attempt.current += 1;
          pending.current = false;
          playAllowed.current = false;
          clearPlayTimer();
          setStarting(false);
          setPlaying(false);
          setPlayError(true);
        }}
      />
      <button type="button" className="session-audio-player__toggle" disabled={disabled || busy}
        aria-label={buttonLabel} title={buttonLabel} aria-busy={busy} onClick={togglePlayback}>
        {busy ? <span className="session-audio-player__spinner" aria-hidden="true" /> : <Icon name={playing ? "pause" : "play"} />}
      </button>
      <div className="session-audio-player__timeline">
        <input type="range" min={0} max={duration ?? 0} step={0.1} value={position}
          disabled={disabled || duration === null} aria-label={labels.seek} aria-valuetext={time}
          onChange={event => {
            const element = audio.current;
            const next = Number(event.currentTarget.value);
            if (disabled || !element || duration === null || !Number.isFinite(next)) return;
            const clamped = Math.min(duration, Math.max(0, next));
            element.currentTime = clamped;
            setPosition(clamped);
          }} />
        <output className="session-audio-player__time" aria-live="off">{time}</output>
      </div>
      {sourceControl && <div className="session-audio-player__source">{sourceControl}</div>}
      {failed && <p className="session-audio-player__error" role="status">{labels.playFailed}</p>}
      {!failed && <span className="settings-sr-only" role="status">{busy ? labels.loading : src && !playing ? labels.ready : ""}</span>}
    </div>
  );
}
