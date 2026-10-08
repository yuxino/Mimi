import { useCallback, useEffect, useRef, useState } from "react";
import { getWindowsLiveCaptionsSupport } from "../../lib/ipc";
import type { WindowsLiveCaptionsSupport } from "../../lib/types";

/** Read-only availability checks never open Live Captions or grant permission. */
export function useWindowsLiveCaptionsSupport(visible: boolean) {
  const [support, setSupport] = useState<WindowsLiveCaptionsSupport | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const generation = useRef(0);
  const refresh = useCallback(async () => {
    const request = ++generation.current;
    setLoading(true);
    setFailed(false);
    try {
      const result = await getWindowsLiveCaptionsSupport();
      if (request === generation.current) setSupport(result);
    } catch {
      if (request === generation.current) { setSupport(null); setFailed(true); }
    } finally {
      if (request === generation.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    let disposed = false;
    if (visible) queueMicrotask(() => { if (!disposed) void refresh(); });
    return () => { disposed = true; generation.current += 1; };
  }, [visible, refresh]);
  return { support, loading, failed, refresh };
}
