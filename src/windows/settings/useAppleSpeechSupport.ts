import { useCallback, useEffect, useRef, useState } from "react";
import { getAppleSpeechSupport } from "../../lib/ipc";
import { I18N } from "../../lib/i18n";
import { activeServiceProfile, textTranslationForProfile } from "../../lib/providerCapabilities";
import { useStore } from "../../lib/store";
import type { AppleSpeechResourceStatus, AppleSpeechSupport, SettingsSnapshot } from "../../lib/types";
import { useSettingsToast } from "./useSettingsToast";

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
  const currentSignature = useRef(readySignature);
  useEffect(() => { currentSignature.current = readySignature; }, [readySignature]);
  const [support, setSupport] = useState<AppleSpeechSupport | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [settledSignature, setSettledSignature] = useState<string | null>();
  const generation = useRef(0);
  const { beginToast } = useSettingsToast();
  const load = useCallback((notify?: (message: string, failure: boolean) => void) => {
    const request = ++generation.current;
    return getAppleSpeechSupport().then(result => {
      if (request === generation.current) { setSupport(result); setFailed(false); setSettledSignature(readySignature); }
    }).catch(() => {
      if (request === generation.current) {
        setSupport(null);
        setFailed(true);
        setSettledSignature(readySignature);
        notify?.(I18N.settings.appleSpeechLoadFailed, true);
      }
    }).finally(() => {
      if (request === generation.current) setLoading(false);
    });
  }, [readySignature]);
  const refresh = useCallback(() => {
    setLoading(true);
    setFailed(false);
    return load(beginToast());
  }, [beginToast, load]);
  useEffect(() => {
    if (visible) void load();
    return () => { generation.current += 1; };
  }, [visible, load, readySignature]);
  const update = useCallback((result: AppleSpeechSupport) => {
    generation.current += 1;
    setSupport(result);
    setLoading(false);
    setFailed(false);
    setSettledSignature(currentSignature.current);
  }, []);
  const current = settledSignature === readySignature;
  return { support, loading: loading || !current, failed: failed && current, refresh, update };
}
