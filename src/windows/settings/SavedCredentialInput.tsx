import { useCallback, useEffect, useRef, useState, type ComponentProps } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { TauriEvent } from "@tauri-apps/api/event";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { isTauri, profileRevealCredential, type StoredCredentialField } from "../../lib/ipc";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import type { TextTranslation } from "../../lib/types";
import { ConfigInput } from "./ConfigInput";
import { useSettingsToast } from "./useSettingsToast";

type Props = Omit<ComponentProps<typeof ConfigInput>, "value" | "action" | "expandable"> & {
  profileId: string;
  field: StoredCredentialField;
  textTranslation?: Exclude<TextTranslation, "followService">;
  label: string;
  hasSavedValue: boolean;
  active?: boolean;
  value: string;
};

/** Saved values are read only on request and remain local to this input. Only an
 * actual edit or paste changes the owning editor's replacement draft. */
export function SavedCredentialInput(props: Props) {
  // These boundaries discard the private component state before rendering the
  // next field. The replacement draft remains owned by the editor.
  return <CredentialInput key={`${props.profileId}:${props.field}:${props.textTranslation ?? ""}:${props.hasSavedValue}:${!!props.disabled}:${props.active !== false}`} {...props} />;
}

function CredentialInput({ profileId, field, textTranslation, label, hasSavedValue, active = true, value, onValueChange, onGroupBlur, onKeyDown, disabled, ...inputProps }: Props) {
  const [preview, setPreview] = useState<{ status: "hidden" | "loading" | "shown"; saved: string | null; draft: string }>({ status: "hidden", saved: null, draft: value });
  const { beginToast, clearToast } = useSettingsToast();
  const mounted = useRef(false);
  const request = useRef(0);
  const hide = useCallback(() => {
    request.current += 1;
    setPreview(current => ({ ...current, status: "hidden", saved: null }));
    clearToast();
  }, [clearToast]);

  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    const hidden = () => { if (document.hidden) hide(); };
    window.addEventListener("blur", hide);
    document.addEventListener("visibilitychange", hidden);
    if (isTauri) {
      try {
        const nativeWindow = getCurrentWindow();
        // Listen directly: onCloseRequested would destroy the hidden settings
        // WebView after our callback, conflicting with native close handling.
        for (const event of [TauriEvent.WINDOW_BLUR, TauriEvent.WINDOW_CLOSE_REQUESTED]) {
          void nativeWindow.listen(event, hide).then(unlisten => {
            if (disposed) unlisten(); else unlisteners.push(unlisten);
          }).catch(() => { /* DOM blur/visibility remain a browser fallback. */ });
        }
      } catch { /* Browser fixtures have no native window. */ }
    }
    return () => {
      disposed = true;
      mounted.current = false;
      request.current += 1;
      window.removeEventListener("blur", hide);
      document.removeEventListener("visibilitychange", hidden);
      unlisteners.forEach(unlisten => unlisten());
    };
  }, [hide]);

  useEffect(() => {
    // Also reject a read when the owner replaces/resets the draft directly.
    request.current += 1;
  }, [value]);
  if (preview.draft !== value) setPreview({ status: "hidden", saved: null, draft: value });

  const reveal = () => {
    if (disabled || !active) return;
    if (preview.status !== "hidden") { hide(); return; }
    clearToast();
    if (value) {
      setPreview({ status: "shown", saved: null, draft: value });
      return;
    }
    if (!hasSavedValue) return;
    const nonce = ++request.current;
    const notify = beginToast();
    setPreview({ status: "loading", saved: null, draft: value });
    void profileRevealCredential({ profileId, field, ...(textTranslation ? { textTranslation } : {}) })
      .then(saved => {
        if (!mounted.current || request.current !== nonce) return;
        setPreview(saved ? { status: "shown", saved, draft: value } : { status: "hidden", saved: null, draft: value });
        if (!saved) notify(I18N.settings.savedCredentialMissing, true);
      })
      .catch((error: unknown) => {
        if (!mounted.current || request.current !== nonce) return;
        setPreview({ status: "hidden", saved: null, draft: value });
        notify(profileErrorMessage(error), true);
      });
  };
  const shown = active && !disabled && preview.status === "shown";
  const actionText = preview.status === "loading" ? I18N.settings.readingSavedCredential
    : value ? shown ? I18N.settings.hideCredential : I18N.settings.showCredential
    : shown ? I18N.settings.hideSavedCredential : I18N.settings.revealSavedCredential;

  return <ConfigInput {...inputProps} expandable={false} disabled={disabled || !active}
    value={value || (shown && hasSavedValue ? preview.saved ?? "" : "")} type={shown ? "text" : "password"}
    placeholder={hasSavedValue && !value ? "••••••••" : inputProps.placeholder}
    aria-description={inputProps["aria-description"] ?? (hasSavedValue && !value ? I18N.settings.savedCredential : undefined)}
    onValueChange={next => {
      request.current += 1;
      setPreview(current => ({ status: current.status === "loading" ? "hidden" : current.status, saved: null, draft: next }));
      clearToast();
      onValueChange(next);
    }}
    onGroupBlur={event => {
      if (!event.currentTarget.contains(event.relatedTarget)) hide();
      onGroupBlur?.(event);
    }}
    onKeyDown={event => {
      if (event.key === "Escape") { event.preventDefault(); hide(); }
      onKeyDown?.(event);
    }}
    action={(hasSavedValue || !!value) && <button type="button" className="saved-credential-input__toggle"
      disabled={disabled || !active} aria-label={`${actionText}: ${label}`} title={`${actionText}: ${label}`}
      aria-pressed={shown} aria-busy={preview.status === "loading" || undefined} onClick={reveal}
      onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); hide(); } }}>
      <Icon name={preview.status === "hidden" ? "eye" : "eye-off"} /><span>{actionText}</span>
    </button>}
  />;
}
