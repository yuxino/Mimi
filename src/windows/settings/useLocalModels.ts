import { useCallback, useEffect, useRef, useState } from "react";
import { localModelsStatus } from "../../lib/ipc";
import type { LocalModelsSnapshot } from "../../lib/types";

export function useLocalModels(visible = true) {
  const [snapshot, setSnapshot] = useState<LocalModelsSnapshot | null>(null);
  const [failed, setFailed] = useState(false);
  const mounted = useRef(false);
  const request = useRef(0);
  const refresh = useCallback(async () => {
    const revision = ++request.current;
    try {
      const status = await localModelsStatus();
      if (mounted.current && revision === request.current) { setSnapshot(status); setFailed(false); }
    } catch { if (mounted.current && revision === request.current) setFailed(true); }
  }, []);
  const invalidate = useCallback(() => { mounted.current = false; request.current++; }, []);
  const busy = snapshot?.models.some(model => ["downloading", "verifying", "cancelling", "deleting"].includes(model.phase));
  useEffect(() => {
    mounted.current = true;
    if (!visible) return invalidate;
    void Promise.resolve().then(refresh);
    const timer = window.setInterval(() => void refresh(), busy ? 500 : 5000);
    const update = () => void refresh();
    window.addEventListener("local-models-updated", update);
    return () => { invalidate(); clearInterval(timer); window.removeEventListener("local-models-updated", update); };
  }, [visible, busy, refresh, invalidate]);
  return { snapshot, failed, refresh };
}
export function refreshLocalModels() { window.dispatchEvent(new Event("local-models-updated")); }
