import { useEffect, useRef, useState, type ComponentProps } from "react";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { LOCAL_MODEL_COPY as copy, localModelError } from "../../lib/localModelI18n";
import { LOCAL_PROGRAM_COPY } from "../../lib/localProgramI18n";
import { cancelLocalModelDownload, deleteLocalModel, downloadLocalModel } from "../../lib/ipc";
import type { LocalModelStatus, LocalSpeechModel } from "../../lib/types";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";
import { SettingsConfirmation } from "./DestructiveConfirmation";
import { InlineFeedback } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";
import { refreshLocalModels, useLocalModels } from "./useLocalModels";
import "./local-models.css";

function modelSize(bytes: number) { return bytes >= 1_000_000_000 ? `${(bytes / 1_000_000_000).toFixed(1)} GB` : `${Math.ceil(bytes / 1_000_000)} MB`; }
const busy = (model: LocalModelStatus) => ["downloading", "verifying", "cancelling", "deleting"].includes(model.phase);
export function LocalModelLibrary({ only, visible = true, disabled = false, onUse, onUseOwn }: { only?: LocalSpeechModel; visible?: boolean; disabled?: boolean; onUse?: (model: LocalSpeechModel, name: string) => Promise<void>; onUseOwn?: () => void }) {
  const { snapshot, failed, refresh } = useLocalModels(visible);
  const [deleting, setDeleting] = useState<LocalModelStatus | null>(null);
  const [pending, setPending] = useState<LocalSpeechModel | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const previous = useRef(new Map<LocalSpeechModel, string>());
  const { beginToast } = useSettingsToast();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    for (const model of snapshot?.models ?? []) {
      if (["downloading", "verifying"].includes(previous.current.get(model.id) ?? "") && model.phase === "installed") beginToast()(copy.ready);
      previous.current.set(model.id, model.phase);
    }
  }, [snapshot, beginToast]);
  const run = async (id: LocalSpeechModel, action: () => Promise<void>, success?: string) => {
    if (inFlight.current) return;
    inFlight.current = true; setPending(id); setError(null);
    const notify = beginToast();
    try { await action(); refreshLocalModels(); await refresh(); if (success) notify(success); }
    catch (failure) { if (mounted.current) setError(localModelError(failure)); }
    finally { inFlight.current = false; if (mounted.current) { setPending(null); setDeleting(null); } }
  };
  return <section className="local-models" aria-label={copy.title}>
    <p className="local-models__privacy">{copy.privacy}</p>
    {failed && <InlineFeedback tone="error">{copy.loadFailed} <button type="button" className="settings-link" onClick={() => void refresh()}>{copy.retryStatus}</button></InlineFeedback>}
    {snapshot && !snapshot.available && <InlineFeedback tone="info">{copy.unavailable}</InlineFeedback>}
    {!snapshot && !failed && <p role="status">{I18N.settings.settingsSnapshotLoading}</p>}
    <div className="local-models__list">
      {snapshot?.models.filter(model => (!only || model.id === only) && ((model.available ?? snapshot.available) || model.installed || busy(model) || model.id === only)).map(model => {
        const working = busy(model);
        const percent = Math.min(100, Math.floor(model.downloadedBytes / model.downloadBytes * 100));
        const status = working ? copy[model.phase as "downloading" | "verifying" | "cancelling" | "deleting"] : model.inUse ? copy.inUse : model.installed ? copy.installed : copy.missing;
        return <div className="local-models__row" key={model.id}>
          <div className="local-models__info"><strong>{model.name}</strong><span>{model.id === "qwenStandard" ? `${copy.standard} · ` : model.id === "qwenSmall" ? `${copy.light} · ` : ""}{modelSize(model.downloadBytes)}</span>
            <span role="status">{status}{["downloading", "verifying"].includes(model.phase) ? ` · ${percent}%` : ""}</span>
          </div>
          <div className="local-models__actions">
            {working ? model.phase === "downloading" || model.phase === "verifying" ? <button type="button" className="settings-button settings-button--compact" disabled={pending !== null} onClick={() => void run(model.id, () => cancelLocalModelDownload(model.id))}>{I18N.settings.cancel}</button> : null
              : model.installed ? <>
                {onUse && <button type="button" className="settings-button settings-button--primary settings-button--compact" aria-label={`${copy.use}: ${model.name}`} disabled={disabled || pending !== null || !(model.available ?? snapshot.available)} onClick={() => void run(model.id, () => onUse(model.id, model.name))}>{copy.use}</button>}
                <button type="button" className="settings-button settings-button--compact" aria-label={`${copy.delete}: ${model.name}`} disabled={pending !== null || model.inUse} onClick={() => setDeleting(model)}><Icon name="trash" /></button>
              </> : <button type="button" className="settings-button settings-button--primary settings-button--compact" aria-label={`${model.phase === "error" ? copy.retry : copy.download}: ${model.name}`} disabled={!(model.available ?? snapshot.available) || pending !== null} onClick={() => void run(model.id, () => downloadLocalModel(model.id))}><Icon name="download" />{model.phase === "error" ? copy.retry : copy.download}</button>}
          </div>
          {working && <progress aria-label={`${model.name}: ${status}`} max={model.downloadBytes} value={model.downloadedBytes} />}
          {model.error && <InlineFeedback tone="error">{localModelError(model.error)}</InlineFeedback>}
          {model.available === false && <InlineFeedback tone="info">{model.id === "qwenSmall" || model.id === "qwenStandard" ? copy.mlxUnavailable : copy.unavailable}</InlineFeedback>}
        </div>;
      })}
    </div>
    {onUseOwn && <button type="button" className={snapshot?.available === false ? "settings-button settings-button--primary settings-button--compact local-models__own" : "settings-link local-models__own"} disabled={disabled} onClick={onUseOwn}>{LOCAL_PROGRAM_COPY.own}<Icon name="chevron-right" /></button>}
    {error && <InlineFeedback tone="error">{error}</InlineFeedback>}
    {deleting && <SettingsConfirmation message={copy.deleteConfirm} disabled={pending !== null} confirmLabel={copy.delete} onCancel={() => setDeleting(null)} onConfirm={() => void run(deleting.id, () => deleteLocalModel(deleting.id), copy.deleted)}><strong>{deleting.name}</strong></SettingsConfirmation>}
  </section>;
}
export function LocalSpeechSettings(props: ComponentProps<typeof AlibabaCredentialEditor>) {
  return <><LocalModelLibrary only={props.profile.localSpeechModel ?? "qwenSmall"} visible={props.visible} /><AlibabaCredentialEditor {...props} textOnly /></>;
}
