import { useCallback, useRef, useState } from "react";
import { sessionActionErrorMessage } from "../../lib/providers/connectionDiagnostics";
import { I18N } from "../../lib/i18n";

/** A click acknowledges immediately; repeated clicks share the same operation. */
export function useSessionAction() {
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const [failureMessage, setFailureMessage] = useState<string | null>(null);
  const clearFailure = useCallback(() => setFailureMessage(null), []);
  const run = useCallback(async (operation: () => Promise<void>, fallback = I18N.overlay.controlActionFailed) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setPending(true);
    setFailureMessage(null);
    try {
      await operation();
    } catch (error) {
      // Failed resume remains paused, so its cause belongs to this action.
      // Match fixed application labels only; never render raw provider text.
      setFailureMessage(sessionActionErrorMessage(error, fallback));
    } finally {
      inFlight.current = false;
      setPending(false);
    }
  }, []);
  return { pending, failed: failureMessage !== null, failureMessage, run, clearFailure };
}
