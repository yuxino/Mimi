import { useEffect, useState } from "react";
import { isTauri, listenOverlayControlMode, overlayControlGetState, type OverlayControlMode } from "../ipc";

/** Both overlay surfaces observe the native mode; a delayed read cannot undo a newer event. */
export function useOverlayControlMode(previewMode: OverlayControlMode | (() => OverlayControlMode) = "island") {
  const [mode, setMode] = useState<OverlayControlMode>(() => isTauri ? "hidden"
    : typeof previewMode === "function" ? previewMode() : previewMode);
  useEffect(() => {
    if (!isTauri) return;
    let disposed = false;
    let eventRevision = 0;
    let removeListener: (() => void) | undefined;
    void listenOverlayControlMode(nextMode => {
      eventRevision += 1;
      if (!disposed) setMode(nextMode);
    }).then(unlisten => {
      if (disposed) { unlisten(); return; }
      removeListener = unlisten;
      const readRevision = eventRevision;
      void overlayControlGetState().then(nextMode => {
        if (!disposed && eventRevision === readRevision) setMode(nextMode);
      }).catch(() => {});
    }).catch(() => {});
    return () => { disposed = true; removeListener?.(); };
  }, []);
  return [mode, setMode] as const;
}
