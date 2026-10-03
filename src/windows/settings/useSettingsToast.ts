import { useCallback, useEffect, useRef } from "react";
import { TransientToast } from "../../lib/transientToast";

interface Notice { message: string; failure: boolean }
let notice: Notice | null = null;
let generation = 0;
const listeners = new Set<() => void>();
const toast = new TransientToast<Notice>(value => {
  notice = value;
  listeners.forEach(listener => listener());
});
export const subscribeSettingsToast = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
export const settingsToastSnapshot = () => notice;

export function dismissSettingsToast() { generation += 1; toast.clear(); }

/** Reserve the single notification slot at the start of an action. Late results
 * cannot revive feedback after navigation, blur, replacement or unmount. */
export function useSettingsToast() {
  const mounted = useRef(false);
  const ownedGeneration = useRef<number | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (ownedGeneration.current === generation) dismissSettingsToast();
    };
  }, []);
  const beginToast = useCallback(() => {
    dismissSettingsToast();
    const current = generation;
    ownedGeneration.current = current;
    return (message: string, failure = false) => {
      if (mounted.current && current === generation) toast.show({ message, failure }, failure);
    };
  }, []);
  const clearToast = useCallback(() => {
    if (ownedGeneration.current === generation) dismissSettingsToast();
  }, []);
  /** Quiet autosave: the control already shows success; only failure needs a notice. */
  const runWithToast = useCallback(async (action: () => Promise<unknown>, failureMessage: string) => {
    const notify = beginToast();
    try { await action(); return true; }
    catch { notify(failureMessage, true); return false; }
  }, [beginToast]);
  return { beginToast, clearToast, runWithToast };
}
