import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getAppleTranslationStatus, getAppleTranslationSupport, isTauri } from "../../../lib/ipc";
import type { AppleTranslationStatus, AppleTranslationSupport, SourceLanguage, TargetLanguage } from "../../../lib/types";

type PendingQuery = { input: symbol; generation: number; promise: Promise<void> };

export function useAppleTranslationSupport(visible: boolean) {
  const enabled = visible && isTauri;
  const input = useMemo(() => Symbol(enabled ? "visible" : "hidden"), [enabled]);
  const currentInput = useRef(input);
  useEffect(() => { currentInput.current = input; }, [input]);
  const generation = useRef(0);
  const pending = useRef<PendingQuery | null>(null);
  const [result, setResult] = useState<{ input: symbol; support: AppleTranslationSupport | null; failed: boolean } | null>(null);
  const load = useCallback(() => {
    if (!enabled || currentInput.current !== input) return Promise.resolve();
    if (pending.current?.input === input && pending.current.generation === generation.current) return pending.current.promise;
    const request = ++generation.current;
    const promise = getAppleTranslationSupport().then(support => {
      if (request === generation.current) setResult({ input, support, failed: false });
    }).catch(() => {
      if (request === generation.current) setResult({ input, support: null, failed: true });
    }).finally(() => {
      if (pending.current?.promise === promise) pending.current = null;
    });
    pending.current = { input, generation: request, promise };
    return promise;
  }, [enabled, input]);
  const refresh = useCallback(() => {
    if (!enabled || currentInput.current !== input) return Promise.resolve();
    setResult(null);
    return load();
  }, [enabled, input, load]);
  useEffect(() => {
    if (enabled) void load();
    return () => { generation.current += 1; };
  }, [enabled, load]);
  const current = enabled && result?.input === input;
  return { support: current ? result.support : null, loading: enabled && !current, failed: current && result.failed, refresh };
}

/** Every result belongs to an exact visible profile and language pair. */
export function useAppleTranslationStatus(profileId: string, source: SourceLanguage, target: TargetLanguage, enabled: boolean) {
  const key = JSON.stringify([profileId, source, target, enabled]);
  const input = useMemo(() => Symbol(key), [key]);
  const currentInput = useRef(input);
  useEffect(() => { currentInput.current = input; }, [input]);
  const generation = useRef(0);
  const pending = useRef<PendingQuery | null>(null);
  const [result, setResult] = useState<{ input: symbol; status: AppleTranslationStatus | null; failed: boolean } | null>(null);
  const load = useCallback(() => {
    if (!enabled || currentInput.current !== input) return Promise.resolve();
    if (pending.current?.input === input && pending.current.generation === generation.current) return pending.current.promise;
    const request = ++generation.current;
    const promise = getAppleTranslationStatus(source, target).then(status => {
      if (request === generation.current) setResult({ input, status, failed: false });
    }).catch(() => {
      if (request === generation.current) setResult({ input, status: null, failed: true });
    }).finally(() => {
      if (pending.current?.promise === promise) pending.current = null;
    });
    pending.current = { input, generation: request, promise };
    return promise;
  }, [enabled, input, source, target]);
  const refresh = useCallback(() => {
    if (!enabled || currentInput.current !== input) return Promise.resolve();
    setResult(null);
    return load();
  }, [enabled, input, load]);
  useEffect(() => {
    if (enabled) void load();
    return () => { generation.current += 1; };
  }, [enabled, load]);
  const update = useCallback((status: AppleTranslationStatus) => {
    generation.current += 1;
    setResult({ input, status, failed: false });
  }, [input]);
  const current = enabled && result?.input === input;
  return { status: current ? result.status : null, failed: current && result.failed, loading: enabled && !current, refresh, update };
}
