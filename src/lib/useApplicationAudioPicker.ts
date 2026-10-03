import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { applicationAudioCopy, applicationAudioError, type ApplicationSnapshot } from "./applicationAudio";
import { audioInputErrorMessage } from "./audioInput";
import { audioSourceErrorMessage } from "./windowsAudioSource";
import { isTauri } from "./ipc";
import { useStore } from "./store";

/** List only after the user opens or refreshes the picker: mounting either
 * surface must not prompt for capture permission or start audio capture. */
export function useApplicationAudioPicker(disabled = false) {
  const target = useStore(state => state.settings.systemAudioTarget) ?? { kind: "system" as const };
  const ready = useStore(state => state.initializationStatus === "ready");
  const status = useStore(state => state.session.status.kind);
  const switchTarget = useStore(state => state.switchSystemAudioTarget);
  const targetKey = target.kind === "application" ? target.id : "";
  const [snapshot, setSnapshot] = useState<{ targetKey: string; value: ApplicationSnapshot } | null>(null);
  const [loading, setLoading] = useState(false);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<{ targetKey: string; message: string } | null>(null);
  const mounted = useRef(false);
  const listing = useRef(false);
  const switching = useRef(false);
  const text = applicationAudioCopy();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  // An external window can choose a newly opened application. Its new target
  // is not missing merely because this window has an older enumeration.
  if (failure && failure.targetKey !== targetKey) setFailure(null);
  const error = failure?.targetKey === targetKey ? failure.message : null;
  const supported = snapshot?.value.supported ?? (!isTauri || /Mac|Windows/i.test(navigator.userAgent));
  const locked = disabled || !ready || pending || status === "connecting" || status === "stopping";
  const applications = snapshot?.value.supported ? snapshot.value.applications : [];
  const missing = target.kind === "application" && snapshot?.targetKey === targetKey && snapshot.value.supported && !applications.some(app => app.id === target.id);
  const selected = target.kind === "application" ? target.id : "";
  const valueLabel = target.kind === "application" ? `${target.name}${missing ? ` · ${text.missing}` : ""}` : text.all;
  const names = new Map<string, number>();
  for (const app of applications) names.set(app.name, (names.get(app.name) ?? 0) + 1);
  const options = [{ value: "", label: text.all }, ...applications.map(app => ({
    value: app.id,
    label: (names.get(app.name) ?? 0) > 1 && app.id.startsWith("windows:") ? `${app.name} · ${app.id.split(":")[1]}` : app.name,
  }))];

  async function refresh() {
    if (listing.current || locked || !supported) return;
    listing.current = true;
    setLoading(true);
    setFailure(null);
    try {
      const value = isTauri ? await invoke<ApplicationSnapshot>("audio_applications") : { supported: true, applications: [] };
      if (mounted.current) setSnapshot({ targetKey, value });
    } catch {
      if (mounted.current) setFailure({ targetKey, message: text.failed });
    } finally {
      listing.current = false;
      if (mounted.current) setLoading(false);
    }
  }

  async function choose(id: string) {
    if (locked || switching.current || id === selected) return;
    const app = applications.find(app => app.id === id);
    if (id && !app) return;
    switching.current = true;
    setPending(true);
    setFailure(null);
    try {
      await switchTarget(app ? { kind: "application", id: app.id, name: app.name } : { kind: "system" });
    } catch (failure) {
      const message = failure instanceof Error ? failure.message : typeof failure === "string" ? failure : "";
      // The backend persists the requested target before reconnecting. A
      // reconnect failure still belongs to that choice; only a third target
      // selected elsewhere supersedes this operation's error.
      const current = useStore.getState().settings.systemAudioTarget;
      const currentKey = current?.kind === "application" ? current.id : "";
      if (mounted.current && (currentKey === id || currentKey === targetKey)) {
        setFailure({ targetKey: currentKey, message: applicationAudioError(message) ?? audioInputErrorMessage(message) ?? audioSourceErrorMessage(message) ?? text.saveFailed });
      }
    } finally {
      switching.current = false;
      if (mounted.current) setPending(false);
    }
  }

  return { text, selected, valueLabel, options, loading, pending, error, missing, supported, locked,
    empty: snapshot?.value.supported === true && applications.length === 0,
    refresh, choose };
}
