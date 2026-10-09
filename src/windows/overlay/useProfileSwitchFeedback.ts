import { useCallback, useEffect, useRef, useState } from "react";
import { isTauri, listenProfileSwitchFeedback } from "../../lib/ipc";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";

/** Keep rejected selections visible when their control popover closes. */
export function useProfileSwitchFeedback() {
  const latestRequest = useRef(0);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const clearFailure = useCallback(() => setError(null), []);
  useEffect(() => {
    if (!isTauri) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenProfileSwitchFeedback(feedback => {
      if (disposed || !Number.isSafeInteger(feedback.requestId) || feedback.requestId < latestRequest.current
        || typeof feedback.pending !== "boolean" || (feedback.error !== null && typeof feedback.error !== "string")) return;
      latestRequest.current = feedback.requestId;
      setPending(feedback.pending);
      setError(feedback.error);
    }).then(remove => { if (disposed) remove(); else unlisten = remove; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, []);
  return { pending, failed: error !== null, failureMessage: error === null ? null : profileErrorMessage(error), clearFailure };
}
