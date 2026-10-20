import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { applicationAudioCopy, applicationAudioError, usesPlaybackStreams, type ApplicationSnapshot } from "./applicationAudio";
import { applicationIconCache } from "./applicationAudioIcons";
import { audioInputErrorMessage } from "./audioInput";
import { audioSourceErrorMessage } from "./windowsAudioSource";
import { isTauri } from "./ipc";
import { useStore } from "./store";

let latestIconRequest = 0;

function currentTargetKey() {
  const target = useStore.getState().settings.systemAudioTarget;
  return target?.kind === "application" ? target.id : "";
}

/** Selectable lists load on user intent. The chosen application's local icon
 * can refresh silently without permission or capture. Linux only reads already
 * discovered streams; it never initializes discovery in the background. */
export function useApplicationAudioPicker(disabled = false) {
  const target = useStore(state => state.settings.systemAudioTarget) ?? { kind: "system" as const };
  const ready = useStore(state => state.initializationStatus === "ready");
  const status = useStore(state => state.session.status.kind);
  const switchTarget = useStore(state => state.switchSystemAudioTarget);
  const targetKey = target.kind === "application" ? target.id : "";
  const [snapshot, setSnapshot] = useState<{ targetKey: string; value: ApplicationSnapshot } | null>(null);
  const [icons, setIcons] = useState(applicationIconCache.read);
  const [loadingState, setLoading] = useState({ targetKey, ready, active: false });
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<{ targetKey: string; message: string } | null>(null);
  const snapshotRef = useRef(snapshot);
  const snapshotRevision = useRef(0);
  const mounted = useRef(false);
  const generation = useRef({ targetKey, ready, value: 0 });
  const listing = useRef<number | null>(null);
  const switching = useRef(false);
  const lookup = useRef<{ generation: number; request: Promise<ApplicationSnapshot> } | null>(null);
  const readSnapshot = useCallback((cachedOnly = false, owner = generation.current.value) => {
    if (!cachedOnly && lookup.current?.generation === owner) return lookup.current.request;
    const iconRequest = ++latestIconRequest;
    const current = () => owner === generation.current.value && currentTargetKey() === targetKey;
    const request = (isTauri ? (cachedOnly
      ? invoke<ApplicationSnapshot>("audio_applications", { cachedOnly: true })
      : invoke<ApplicationSnapshot>("audio_applications")) : Promise.resolve({ supported: true, applications: [] }))
      .then(value => {
        if (current() && iconRequest === latestIconRequest) applicationIconCache.replace(value);
        return value;
      }, error => {
        if (current() && iconRequest === latestIconRequest) applicationIconCache.clear();
        throw error;
      });
    if (!cachedOnly) {
      lookup.current = { generation: owner, request };
      const complete = () => { if (lookup.current?.request === request) lookup.current = null; };
      void request.then(complete, complete);
    }
    return request;
  }, [targetKey]);
  const text = applicationAudioCopy();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { snapshotRef.current = snapshot; }, [snapshot]);
  useEffect(() => {
    const previous = generation.current;
    if (previous.targetKey !== targetKey || previous.ready !== ready) {
      generation.current = { targetKey, ready, value: previous.value + 1 };
    }
    const owner = generation.current.value;
    listing.current = null;
    if (!isTauri || !/Mac|Windows|Linux/i.test(navigator.userAgent) || !ready || !targetKey) return;
    const cachedOnly = usesPlaybackStreams();
    const snapshot = snapshotRef.current;
    if (cachedOnly && snapshot?.targetKey === targetKey) return;
    // An older list only covers this choice while its icon is still cached.
    // A failed lookup can clear icons before another window selects it again.
    // Completed manual refreshes do not restart this effect: a missing icon or
    // exited target is a valid result, not a reason to enumerate again.
    if (snapshot && applicationIconCache.read().has(targetKey)
      && (snapshot.targetKey === targetKey || snapshot.value.applications.some(app => app.id === targetKey))) return;
    let active = true;
    const revision = ++snapshotRevision.current;
    const current = () => active && revision === snapshotRevision.current
      && owner === generation.current.value && currentTargetKey() === targetKey;
    const restoreIcon = () => { if (current()) setIcons(applicationIconCache.read()); };
    void readSnapshot(cachedOnly, owner).then(value => {
      if (current() && cachedOnly) setSnapshot({ targetKey, value });
      restoreIcon();
    }, restoreIcon);
    return () => { active = false; };
  }, [readSnapshot, ready, targetKey]);
  // Loading belongs to one target visit, including a return to the same app.
  if (loadingState.targetKey !== targetKey || loadingState.ready !== ready) {
    setLoading({ targetKey, ready, active: false });
  }
  const loading = loadingState.targetKey === targetKey && loadingState.ready === ready && loadingState.active;
  // An external window can choose a newly opened application. Its new target
  // is not missing merely because this window has an older enumeration.
  if (failure && failure.targetKey !== targetKey) setFailure(null);
  const error = failure?.targetKey === targetKey ? failure.message : null;
  const supported = snapshot?.value.supported ?? (!isTauri || /Mac|Windows|Linux/i.test(navigator.userAgent));
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
    if (listing.current !== null || locked || !supported) return;
    const owner = generation.current.value;
    const current = () => mounted.current && owner === generation.current.value && currentTargetKey() === targetKey;
    listing.current = owner;
    // A late cached-only result cannot replace this explicit enumeration.
    snapshotRevision.current += 1;
    setLoading({ targetKey, ready, active: true });
    setFailure(null);
    try {
      const value = await readSnapshot(false, owner);
      if (current()) {
        setIcons(applicationIconCache.read());
        setSnapshot({ targetKey, value });
      }
    } catch {
      if (current()) {
        setIcons(applicationIconCache.read());
        if (reportFailure) reportFailure(text.failed);
        else setFailure({ targetKey, message: text.failed });
      }
    } finally {
      if (listing.current === owner) {
        listing.current = null;
        if (current()) setLoading({ targetKey, ready, active: false });
      }
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
