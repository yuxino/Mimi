import { useCallback, useEffect, useRef } from "react";
import { I18N } from "../../../../lib/i18n";
import { speechLanguageGuidance } from "../../../../lib/providers/speechLanguageGuidance";
import type { SettingsSnapshot, TargetLanguage } from "../../../../lib/types";
import { useSettingsToast } from "../../feedback/useSettingsToast";

type Notify = (message: string, failure?: boolean) => void;
interface Pending { before: SettingsSnapshot; target: TargetLanguage; notify: Notify; accepted: boolean }

/** Notify only after the successful action AND its actual settings readback.
 * IPC completion and settings-changed events can arrive in either order. */
export function useLanguageNormalizationToast(settings: SettingsSnapshot) {
  const { beginToast } = useSettingsToast();
  const current = useRef(settings);
  const pending = useRef<Pending | null>(null);
  const expiry = useRef<ReturnType<typeof setTimeout> | null>(null);
  const clear = useCallback(() => {
    pending.current = null;
    if (expiry.current !== null) clearTimeout(expiry.current);
    expiry.current = null;
  }, []);
  const flush = useCallback((after: SettingsSnapshot) => {
    const action = pending.current;
    if (!action) return;
    if (after.activeProfileId !== action.before.activeProfileId) { clear(); return; }
    if (!action.accepted || after.targetLanguage !== action.target) return;
    if (after.sourceLanguage !== action.before.sourceLanguage) {
      action.notify(I18N.settings.recognitionLanguageAdjusted(
        speechLanguageGuidance(action.before).optionLabel(action.before.sourceLanguage),
        speechLanguageGuidance(after).optionLabel(after.sourceLanguage),
      ));
    }
    clear();
  }, [clear]);
  useEffect(() => { current.current = settings; flush(settings); }, [settings, flush]);
  useEffect(() => clear, [clear]);
  return useCallback((target: TargetLanguage | undefined, notify?: Notify) => {
    clear();
    if (target === undefined || target === current.current.targetLanguage) return () => {};
    const action: Pending = { before: current.current, target, notify: notify ?? beginToast(), accepted: false };
    pending.current = action;
    return (accepted: boolean) => {
      if (pending.current !== action) return;
      if (!accepted) { clear(); return; }
      action.accepted = true;
      expiry.current = setTimeout(clear, 5_000);
      flush(current.current);
    };
  }, [beginToast, clear, flush]);
}
