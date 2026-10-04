import { useEffect, useState } from "react";
import { profileCredentialEditorState, type CredentialEditorState } from "../../lib/ipc";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import type { TextTranslation } from "../../lib/types";

/** Editor-local configuration and presence only; secret values use explicit reveal. */
export function useCredentialEditorState(profileId: string, textTranslation?: Exclude<TextTranslation, "followService">, active = true, refreshKey = 0) {
  const key = JSON.stringify([profileId, textTranslation, active, refreshKey]);
  const [result, setResult] = useState<{ key: string; state: CredentialEditorState | null; error: string | null } | null>(null);
  const [renderedKey, setRenderedKey] = useState(key);
  if (renderedKey !== key) {
    setRenderedKey(key);
    setResult(null);
  }
  useEffect(() => {
    if (!active) return;
    let current = true;
    void profileCredentialEditorState({ profileId, ...(textTranslation ? { textTranslation } : {}) })
      .then(state => { if (current) setResult({ key, state, error: null }); })
      .catch((error: unknown) => { if (current) setResult({ key, state: null, error: profileErrorMessage(error) }); });
    return () => { current = false; };
  }, [profileId, textTranslation, active, refreshKey, key]);
  return {
    state: active && result?.key === key ? result.state : null,
    error: active && result?.key === key ? result.error : null,
    loading: active && result?.key !== key,
  };
}
