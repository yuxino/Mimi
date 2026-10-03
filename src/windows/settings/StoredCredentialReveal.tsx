import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { TauriEvent } from "@tauri-apps/api/event";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { isTauri, profileRevealCredential, type StoredCredentialField } from "../../lib/ipc";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import type { TextTranslation } from "../../lib/types";
import { useSettingsToast } from "./useSettingsToast";

/** Mount only while the matching saved field is visible. Reveals never enter a
 * replacement draft, shared store, diagnostics, or persistent WebView storage. */
type Props = {
  profileId: string;
  field: StoredCredentialField;
  textTranslation?: Exclude<TextTranslation, "followService">;
  label: string;
  disabled: boolean;
};

export function StoredCredentialReveal(props: Props) {
  return <SavedCredentialPreview key={`${props.profileId}:${props.field}:${props.textTranslation ?? ""}`} {...props} />;
}

function SavedCredentialPreview({ profileId, field, textTranslation, label, disabled }: Props) {
  const [preview, setPreview] = useState<{ status: "hidden" | "loading" | "shown"; value: string }>({ status: "hidden", value: "" });
  const { beginToast } = useSettingsToast();
  const mounted = useRef(false);
  const request = useRef(0);
  const hide = useCallback(() => {
    request.current += 1;
    setPreview({ status: "hidden", value: "" });
  }, []);
  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let unlistenClose: (() => void) | undefined;
    const visibilityChanged = () => { if (document.hidden) hide(); };
    // Native settings close hides its WebView without unmounting React.
    // Blur also clears a value when another app or an OS prompt takes focus.
    window.addEventListener("blur", hide);
    document.addEventListener("visibilitychange", visibilityChanged);
    if (isTauri) {
      // Observe the event directly: onCloseRequested would call destroy()
      // after our callback and conflict with native hide/normal-exit handling.
      void getCurrentWindow().listen(TauriEvent.WINDOW_CLOSE_REQUESTED, () => { if (mounted.current) hide(); }).then((unlisten) => {
        if (disposed) unlisten(); else unlistenClose = unlisten;
      }).catch(() => { /* Blur/visibility still clear the preview. */ });
    }
    return () => {
      disposed = true;
      mounted.current = false;
      request.current += 1;
      window.removeEventListener("blur", hide);
      document.removeEventListener("visibilitychange", visibilityChanged);
      unlistenClose?.();
    };
  }, [hide]);
  const reveal = () => {
    const nonce = ++request.current;
    const notify = beginToast();
    setPreview({ status: "loading", value: "" });
    void profileRevealCredential({ profileId, field, ...(textTranslation ? { textTranslation } : {}) })
      .then((value) => {
        if (!mounted.current || request.current !== nonce) return;
        setPreview(value ? { status: "shown", value } : { status: "hidden", value: "" });
        if (!value) notify(I18N.settings.savedCredentialMissing, true);
      })
      .catch((error: unknown) => {
        if (!mounted.current || request.current !== nonce) return;
        // Only map known native labels; never display a raw provider/OS error.
        setPreview({ status: "hidden", value: "" });
        notify(profileErrorMessage(error), true);
      });
  };

  return <span className="stored-credential-reveal">
    <button type="button" className="settings-link stored-credential-reveal__toggle" title={`${preview.status === "hidden" ? I18N.settings.revealSavedCredential : I18N.settings.hideSavedCredential}: ${label}`} disabled={disabled && preview.status === "hidden"} aria-label={`${preview.status === "hidden" ? I18N.settings.revealSavedCredential : I18N.settings.hideSavedCredential}: ${label}`} aria-expanded={preview.status !== "hidden"} aria-busy={preview.status === "loading"} onClick={preview.status === "hidden" ? reveal : hide}>
      <Icon name={preview.status === "hidden" ? "eye" : "eye-off"} /><span className="settings-sr-only">{preview.status === "hidden" ? I18N.settings.revealSavedCredential : preview.status === "loading" ? I18N.settings.readingSavedCredential : I18N.settings.hideSavedCredential}</span>
    </button>
    {preview.status === "shown" && <input type="text" readOnly value={preview.value} aria-label={`${label}: ${I18N.settings.savedCredential}`} autoComplete="off" spellCheck={false} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); hide(); } }} />}
  </span>;
}
