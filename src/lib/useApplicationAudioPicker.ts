import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { applicationAudioCopy, applicationAudioError, type ApplicationSnapshot } from "./applicationAudio";
import { applicationIconCache } from "./applicationAudioIcons";
import { audioInputErrorMessage } from "./audioInput";
import { audioSourceErrorMessage } from "./windowsAudioSource";
import { isTauri } from "./ipc";
import { useStore } from "./store";

let latestIconRequest = 0;

/** Selectable lists load on user intent. The chosen application's local icon
 * can refresh silently on macOS/Windows, without permission or capture. */
export function useApplicationAudioPicker(disabled = false) {
  const target = useStore(state => state.settings.systemAudioTarget) ?? { kind: "system" as const };
  const ready = useStore(state => state.initializationStatus === "ready");
  const status = useStore(state => state.session.status.kind);
  const switchTarget = useStore(state => state.switchSystemAudioTarget);
  const targetKey = target.kind === "application" ? target.id : "";
  const [snapshot, setSnapshot] = useState<{ targetKey: string; value: ApplicationSnapshot } | null>(null);
  const [icons, setIcons] = useState(applicationIconCache.read);
  const [loading, setLoading] = useState(false);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<{ targetKey: string; message: string } | null>(null);
  const mounted = useRef(false);
  const listing = useRef(false);
  const switching = useRef(false);
  const lookup = useRef<Promise<ApplicationSnapshot> | null>(null);
  const readSnapshot = useCallback(() => {
    if (lookup.current) return lookup.current;
    const generation = ++latestIconRequest;
    const request = (isTauri ? invoke<ApplicationSnapshot>("audio_applications") : Promise.resolve({ supported: true, applications: [] }))
      .then(value => {
        if (generation === latestIconRequest) applicationIconCache.replace(value);
        return value;
      }, error => {
        if (generation === latestIconRequest) applicationIconCache.clear();
        throw error;
      });
    lookup.current = request;
    void request.then(() => { lookup.current = null; }, () => { lookup.current = null; });
    return request;
  }, []);
  const text = applicationAudioCopy();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    if (!isTauri || !/Mac|Windows/i.test(navigator.userAgent) || !ready || !targetKey) return;
    // An older list only covers this choice while its icon is still cached.
    // A failed lookup can clear icons before another window selects it again.
    // Cache updates are not dependencies: missing icons never start a retry loop.
    if (snapshot && applicationIconCache.read().has(targetKey)
      && (snapshot.targetKey === targetKey || snapshot.value.applications.some(app => app.id === targetKey))) return;
    let active = true;
    const restoreIcon = () => { if (active) setIcons(applicationIconCache.read()); };
    void readSnapshot().then(restoreIcon, restoreIcon);
    return () => { active = false; };
  }, [readSnapshot, ready, snapshot, targetKey]);
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
  const options = [{ value: "", label: text.all, iconDataUrl: null }, ...applications.map(app => ({
    value: app.id,
    iconDataUrl: icons.get(app.id),
    label: (names.get(app.name) ?? 0) > 1 && app.id.startsWith("windows:") ? `${app.name} · ${app.id.split(":")[1]}` : app.name,
  }))];

  async function refresh(reportFailure?: (message: string) => void) {
    if (listing.current || locked || !supported) return;
    listing.current = true;
    setLoading(true);
    setFailure(null);
    try {
      const value = await readSnapshot();
      if (mounted.current) {
        setIcons(applicationIconCache.read());
        setSnapshot({ targetKey, value });
      }
    } catch {
      if (mounted.current) {
        setIcons(applicationIconCache.read());
        if (reportFailure) reportFailure(text.failed);
        else setFailure({ targetKey, message: text.failed });
      }
    } finally {
      listing.current = false;
      if (mounted.current) setLoading(false);
    }
  }

  async function choose(id: string, reportFailure?: (message: string) => void) {
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
        const safeMessage = applicationAudioError(message) ?? audioInputErrorMessage(message) ?? audioSourceErrorMessage(message) ?? text.saveFailed;
        if (reportFailure) reportFailure(safeMessage);
        else setFailure({ targetKey: currentKey, message: safeMessage });
      }
    } finally {
      switching.current = false;
      if (mounted.current) setPending(false);
    }
  }

  return { text, selected, valueLabel, selectedIconDataUrl: icons.get(selected), options, loading, pending, error, missing, supported, locked,
    empty: snapshot?.value.supported === true && applications.length === 0,
    refresh, choose };
}
