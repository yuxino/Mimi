import { useEffect, useRef, useState } from "react";
import { sessionArchiveAudio, sessionHistoryAudio } from "../../../lib/ipc";
import { useStore } from "../../../lib/store";
import type { AudioSource } from "../../../lib/types";

interface SessionAudioPreviewContext {
  active: boolean;
  pageVisible: boolean;
  readerOpen: boolean;
  selectedId: string | null;
  audioSource: AudioSource | undefined;
  recordAudio: boolean;
}

/** Owns the reader's native file transfer and Blob URL. Selection, source and
 * reader-navigation actions clear the preview synchronously before changing owner. */
export function useSessionAudioPreview({
  active,
  pageVisible,
  readerOpen,
  selectedId,
  audioSource,
  recordAudio,
}: SessionAudioPreviewContext) {
  const [url, setUrl] = useState<string | null>(null);
  const [readError, setReadError] = useState(false);
  const [loading, setLoading] = useState(false);
  const requestGeneration = useRef(0);
  const inFlight = useRef(false);
  const lifetime = useRef(0);
  const [renderedContext, setRenderedContext] = useState({ active, pageVisible, selectedId, recordAudio });

  if (renderedContext.active !== active || renderedContext.pageVisible !== pageVisible
    || renderedContext.selectedId !== selectedId || renderedContext.recordAudio !== recordAudio) {
    setRenderedContext({ active, pageVisible, selectedId, recordAudio });
    if (active || !pageVisible || (!selectedId && !recordAudio)) {
      setUrl(null);
      setReadError(false);
    }
  }

  useEffect(() => {
    lifetime.current += 1;
    return () => {
      lifetime.current += 1;
      requestGeneration.current += 1;
    };
  }, []);

  useEffect(() => {
    if (active || !pageVisible || (!selectedId && !recordAudio)) {
      requestGeneration.current += 1;
    }
  }, [active, pageVisible, selectedId, recordAudio]);

  useEffect(() => {
    return () => { if (url) URL.revokeObjectURL(url); };
  }, [url]);

  function clear() {
    requestGeneration.current += 1;
    setUrl(null);
    setReadError(false);
  }

  function load() {
    // One WAV may be 64 MiB. Retiring its result must not start another
    // full-file transfer while the native read is still in flight.
    if (inFlight.current || !pageVisible || !readerOpen || useStore.getState().session.isActive) return;
    inFlight.current = true;
    const currentLifetime = lifetime.current;
    const request = ++requestGeneration.current;
    setLoading(true);
    setReadError(false);
    const read = selectedId ? sessionHistoryAudio(selectedId, audioSource) : sessionArchiveAudio(audioSource);
    void read.then(
      (bytes) => {
        if (request !== requestGeneration.current || useStore.getState().session.isActive) return;
        setUrl(URL.createObjectURL(new Blob([bytes], { type: "audio/wav" })));
        setReadError(false);
      },
      () => { if (request === requestGeneration.current) setReadError(true); },
    ).finally(() => {
      inFlight.current = false;
      if (lifetime.current === currentLifetime) setLoading(false);
    });
  }

  return { url, readError, loading, load, clear };
}
