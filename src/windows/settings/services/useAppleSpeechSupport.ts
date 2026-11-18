import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getAppleSpeechSupport } from "../../../lib/ipc";
import { I18N } from "../../../lib/i18n";
import { activeServiceProfile, textTranslationForProfile } from "../../../lib/providers/providerCapabilities";
import { useStore } from "../../../lib/store";
import type { AppleSpeechResourceStatus, AppleSpeechSupport, SettingsSnapshot } from "../../../lib/types";
import { useSettingsToast } from "../useSettingsToast";

export function appleSpeechLanguageStatus(language: AppleSpeechSupport["languages"][number] | undefined): AppleSpeechResourceStatus {
  if (language?.status) return language.status;
  // Legacy booleans can establish readiness or an ongoing download, but false
  // never distinguishes a missing pack from an unconfirmed native module.
  if (language?.installed) return "installed";
  if (language?.downloading) return "downloading";
  return "unknown";
}

function readyLanguagesSignature(settings: SettingsSnapshot): string | null {
  const native = settings.languageCapabilities;
  if (!native || native.provider !== "appleSpeech") return null;
  const profile = activeServiceProfile(settings);
  if (!profile || profile.provider !== native.provider || profile.id !== native.profileId
    || textTranslationForProfile(profile) !== native.textTranslation || settings.targetLanguage !== native.targetLanguage
    || !Array.isArray(native.sourceLanguages)) return null;
  // The full resource revision also changes when a route-filtered language loses
  // assets. Older payloads fall back to the ready list; neither infers installation.
  // Equivalent query broadcasts must not trigger another query by object identity.
  const revision = native.appleSpeechSupportRevision;
  const resources = typeof revision === "number" && Number.isSafeInteger(revision) && revision >= 0
    ? revision : [...new Set(native.sourceLanguages)].sort();
  return JSON.stringify([native.profileId, native.provider, native.textTranslation, native.targetLanguage,
    resources]);
}

export function useAppleSpeechSupport(visible: boolean) {
  const readySignature = useStore(state => readyLanguagesSignature(state.settings));
  const visit = useMemo(() => Symbol(visible ? "visible" : "hidden"), [visible]);
  const context = useMemo(() => ({ readySignature, visible, visit }), [readySignature, visible, visit]);
  const currentContext = useRef(context);
  useEffect(() => { currentContext.current = context; }, [context]);
  const [support, setSupport] = useState<AppleSpeechSupport | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [settledContext, setSettledContext] = useState<typeof context>();
  const generation = useRef(0);
  const { beginToast } = useSettingsToast();
  const load = useCallback((notify?: (message: string, failure: boolean) => void) => {
    if (!context.visible || currentContext.current !== context) return Promise.resolve();
    const request = ++generation.current;
    return getAppleSpeechSupport().then(result => {
      if (request === generation.current) { setSupport(result); setFailed(false); setSettledContext(context); }
    }).catch(() => {
      if (request === generation.current) {
        setSupport(null);
        setFailed(true);
        setSettledContext(context);
        notify?.(I18N.settings.appleSpeechLoadFailed, true);
      }
    }).finally(() => {
      if (request === generation.current) setLoading(false);
    });
  }, [context]);
  const refresh = useCallback(() => {
    if (!context.visible || currentContext.current !== context) return Promise.resolve();
    setLoading(true);
    setFailed(false);
    return load(beginToast());
  }, [beginToast, context, load]);
  useEffect(() => {
    if (visible) void load();
    return () => { generation.current += 1; };
  }, [visible, load]);
  const update = useCallback((result: AppleSpeechSupport) => {
    generation.current += 1;
    setSupport(result);
    setLoading(false);
    setFailed(false);
    setSettledContext(currentContext.current);
  }, []);
  const current = settledContext === context;
  // Keep in-place refreshes mounted but disabled; only a new visible visit
  // must withhold its previous visit's readiness until the fresh query settles.
  return { support: visible && settledContext?.visit !== visit ? null : support, loading: loading || !current, failed: failed && current, refresh, update };
}
