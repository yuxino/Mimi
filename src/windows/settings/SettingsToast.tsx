import { useEffect, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { AlertCircle, Check, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { TauriEvent } from "@tauri-apps/api/event";
import { effectiveUiLanguage } from "../../lib/i18n";
import { isTauri } from "../../lib/ipc";
import { dismissSettingsToast, settingsToastSnapshot, subscribeSettingsToast } from "./useSettingsToast";
import "./settings-toast.css";

/** Mount once inside the settings theme root, outside the scrolling panels. */
export function SettingsToastRegion({ scopeKey }: { scopeKey?: string }) {
  const current = useSyncExternalStore(subscribeSettingsToast, settingsToastSnapshot);
  const [dialog, setDialog] = useState<HTMLElement | null>(null);
  useEffect(() => {
    // Modal backgrounds are inert. Put the same single toast inside the active
    // dialog so failed create/delete actions stay visible and dismissible.
    const syncDialog = () => setDialog(document.querySelector<HTMLElement>(".settings-confirmation"));
    syncDialog();
    const observer = new MutationObserver(syncDialog);
    observer.observe(document.body, { childList: true });
    return () => observer.disconnect();
  }, []);
  useEffect(() => { dismissSettingsToast(); return dismissSettingsToast; }, [scopeKey]);
  useEffect(() => {
    let disposed = false;
    const nativeUnlisteners: (() => void)[] = [];
    const hidden = () => { if (document.hidden) dismissSettingsToast(); };
    window.addEventListener("hashchange", dismissSettingsToast);
    window.addEventListener("blur", dismissSettingsToast);
    document.addEventListener("visibilitychange", hidden);
    if (isTauri) {
      try {
        const nativeWindow = getCurrentWindow();
        for (const event of [TauriEvent.WINDOW_BLUR, TauriEvent.WINDOW_CLOSE_REQUESTED]) {
          void nativeWindow.listen(event, dismissSettingsToast).then(unlisten => {
            if (disposed) unlisten(); else nativeUnlisteners.push(unlisten);
          }).catch(() => { /* DOM lifecycle events remain a browser fallback. */ });
        }
      } catch { /* A browser fixture has no native window. */ }
    }
    return () => {
      disposed = true;
      nativeUnlisteners.forEach(unlisten => unlisten());
      window.removeEventListener("hashchange", dismissSettingsToast);
      window.removeEventListener("blur", dismissSettingsToast);
      document.removeEventListener("visibilitychange", hidden);
    };
  }, []);
  if (!current) return null;
  const close = { zh: "关闭提示", en: "Dismiss notification", ja: "通知を閉じる" }[effectiveUiLanguage()];
  const notification = <div className="settings-toast" data-tone={current.failure ? "error" : "success"} role={current.failure ? "alert" : "status"} aria-live={current.failure ? "assertive" : "polite"} aria-atomic="true">
    {current.failure ? <AlertCircle size={16} aria-hidden="true" /> : <Check size={16} aria-hidden="true" />}
    <span>{current.message}</span>
    <button type="button" aria-label={close} onClick={dismissSettingsToast}><X size={16} aria-hidden="true" /></button>
  </div>;
  return dialog ? createPortal(notification, dialog) : notification;
}
