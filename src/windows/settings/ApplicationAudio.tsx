import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "../../lib/ipc";
import { applicationAudioCopy, type ApplicationSnapshot } from "../../lib/applicationAudio";
import { useStore } from "../../lib/store";
import { Select } from "../../components/Select";
import { Icon } from "../../components/Icon";
import { InlineFeedback, SettingsRow } from "./SettingsPrimitives";

export function ApplicationAudio() {
  const target = useStore(state => state.settings.systemAudioTarget) ?? { kind: "system" as const };
  const active = useStore(state => state.session.isActive);
  const paused = useStore(state => state.session.isPaused);
  const status = useStore(state => state.session.status.kind);
  const ready = useStore(state => state.initializationStatus) === "ready";
  const save = useStore(state => state.saveSettings);
  const [browsing, setBrowsing] = useState(false);
  const [snapshot, setSnapshot] = useState<ApplicationSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState<"list" | "save" | null>(null);
  const mounted = useRef(false);
  const inFlight = useRef(false);
  const listing = useRef(false);
  const text = applicationAudioCopy();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const requiresStop = active || paused || status === "connecting" || status === "stopping";
  const disabled = !ready || requiresStop || saving;
  // The initial OS hint avoids querying ScreenCaptureKit (and prompting for
  // permission) just because the user opened Settings. Native results win.
  const supported = snapshot?.supported ?? (!isTauri || /Mac|Windows/i.test(navigator.userAgent));
  const applicationMode = target.kind === "application" || browsing;
  const selected = target.kind === "application" ? target.id : "";
  const missing = target.kind === "application" && snapshot?.supported === true && !snapshot.applications.some(app => app.id === target.id);
  async function refresh() {
    if (listing.current || !ready) return;
    listing.current = true; setLoading(true); setFailed(null);
    try {
      const value = isTauri ? await invoke<ApplicationSnapshot>("audio_applications") : { supported: true, applications: [] };
      if (mounted.current) setSnapshot(value);
    } catch { if (mounted.current) setFailed("list"); }
    finally { listing.current = false; if (mounted.current) setLoading(false); }
  }
  async function choose(id: string) {
    if (disabled || inFlight.current) return;
    const app = snapshot?.applications.find(app => app.id === id);
    if (id && !app) return;
    inFlight.current = true; setSaving(true); setFailed(null);
    try {
      await save({ systemAudioTarget: app ? { kind: "application", id: app.id, name: app.name } : { kind: "system" } });
      if (mounted.current) setBrowsing(false);
    } catch { if (mounted.current) setFailed("save"); }
    finally { inFlight.current = false; if (mounted.current) setSaving(false); }
  }
  return <>
    <SettingsRow label={text.title} description={text.help} hint={requiresStop ? text.stop : !supported ? text.unsupported : undefined}>
      <Select label={text.title} value={applicationMode ? "application" : "system"} disabled={disabled}
        options={[{ value: "system", label: text.all }, ...(supported || target.kind === "application" ? [{ value: "application", label: text.app }] : [])]}
        onChange={value => {
          if (value === "system") { if (target.kind === "system") setBrowsing(false); else void choose(""); }
          else { setBrowsing(true); void refresh(); }
        }} />
    </SettingsRow>
    {applicationMode && <SettingsRow label={text.application} description={text.help}>
      <span className="application-audio-controls">
        <Select label={text.application} searchLabel={text.search} emptyMessage={text.noMatch}
          value={selected} disabled={disabled || loading || !supported}
          options={[
            ...(!selected ? [{ value: "", label: loading ? text.loading : text.choose }] : []),
            ...(target.kind === "application" && !snapshot?.applications.some(app => app.id === target.id)
              ? [{ value: target.id, label: `${target.name}${missing ? ` · ${text.missing}` : ""}` }] : []),
            ...(snapshot?.applications.map(app => ({ value: app.id, label: snapshot.applications.filter(other => other.name === app.name).length > 1 && app.id.startsWith("windows:") ? `${app.name} · ${app.id.split(":")[1]}` : app.name, icon: <Icon name="app-window" /> })) ?? []),
          ]} onChange={id => { if (id) void choose(id); }} />
        <button className="settings-button" type="button" disabled={!ready || loading || saving} onClick={() => void refresh()}>{text.refresh}</button>
      </span>
    </SettingsRow>}
    {applicationMode && loading && <InlineFeedback tone="info">{text.loading}</InlineFeedback>}
    {applicationMode && missing && <InlineFeedback tone="error">{text.unavailable}</InlineFeedback>}
    {applicationMode && !loading && snapshot?.supported && snapshot.applications.length === 0 && <InlineFeedback tone="info">{text.empty}</InlineFeedback>}
    {applicationMode && snapshot?.supported === false && <InlineFeedback tone="info">{text.unsupported}</InlineFeedback>}
    {failed && <InlineFeedback tone="error">{failed === "list" ? text.failed : text.saveFailed}</InlineFeedback>}
  </>;
}
