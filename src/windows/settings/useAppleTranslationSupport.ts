import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getAppleTranslationStatus, getAppleTranslationSupport, isTauri } from "../../lib/ipc";
import type { AppleTranslationStatus, AppleTranslationSupport, SourceLanguage, TargetLanguage } from "../../lib/types";

export function useAppleTranslationSupport(visible: boolean) {
  const enabled = visible && isTauri;
  const input = useMemo(() => Symbol(enabled ? "visible" : "hidden"), [enabled]);
  const generation = useRef(0);
  const [result, setResult] = useState<{ input: symbol; support: AppleTranslationSupport | null; failed: boolean } | null>(null);
  const load = useCallback(() => {
    const request = ++generation.current;
    return getAppleTranslationSupport().then(support => {
      if (request === generation.current) setResult({ input, support, failed: false });
    }).catch(() => {
      if (request === generation.current) setResult({ input, support: null, failed: true });
    });
  }, [input]);
  useEffect(() => {
    if (enabled) void load();
    return () => { generation.current += 1; };
  }, [enabled, load]);
  const current = enabled && result?.input === input;
  return { support: current ? result.support : null, loading: enabled && !current, failed: current && result.failed, refresh: load };
}

/** Every result belongs to an exact visible profile and language pair. */
export function useAppleTranslationStatus(profileId: string, source: SourceLanguage, target: TargetLanguage, enabled: boolean) {
  const key = JSON.stringify([profileId, source, target, enabled]);
  const input = useMemo(() => Symbol(key), [key]);
  const generation = useRef(0);
  const [result, setResult] = useState<{ input: symbol; status: AppleTranslationStatus | null; failed: boolean } | null>(null);
  const load = useCallback(() => {
    const request = ++generation.current;
    return getAppleTranslationStatus(source, target).then(status => {
      if (request === generation.current) setResult({ input, status, failed: false });
    }).catch(() => {
      if (request === generation.current) setResult({ input, status: null, failed: true });
    });
  }, [input, source, target]);
  useEffect(() => {
    if (enabled) void load();
    return () => { generation.current += 1; };
  }, [enabled, load]);
  const update = useCallback((status: AppleTranslationStatus) => {
    generation.current += 1;
    setResult({ input, status, failed: false });
  }, [input]);
  const current = enabled && result?.input === input;
  return { status: current ? result.status : null, failed: current && result.failed, loading: enabled && !current, refresh: load, update };
}
