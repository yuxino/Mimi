import { Icon } from "../../components/Icon";
import { useEffect, useRef, useState, type ComponentProps } from "react";
import { I18N } from "../../lib/i18n";
import { LOCAL_PROGRAM_COPY as copy, localProgramError } from "../../lib/localProgramI18n";
import { pickLocalProgramPath } from "../../lib/ipc";
import type { LocalProgramConfiguration, LocalProgramEngine } from "../../lib/types";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";
import { ConfigInput } from "./ConfigInput";
import { SettingsHelp } from "./SettingsHelp";
import { InlineFeedback, SettingsSelect } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";
import "./local-models.css";

const empty = (): LocalProgramConfiguration => ({ engine: "whisperCpp", executable: "", modelPath: "", arguments: [] });
export function LocalProgramSettings(props: ComponentProps<typeof AlibabaCredentialEditor> & { onSaveProgram: (config: LocalProgramConfiguration) => Promise<void> }) {
  const [draft, setDraft] = useState(() => props.profile.localProgram ?? empty());
  const [argumentsText, setArgumentsText] = useState(() => draft.arguments.join("\n"));
  const [savedKey, setSavedKey] = useState(JSON.stringify(props.profile.localProgram));
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(false);
  const inFlight = useRef(false);
  const { beginToast } = useSettingsToast();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const currentKey = JSON.stringify(props.profile.localProgram);
  if (savedKey !== currentKey) { const next = props.profile.localProgram ?? empty(); setSavedKey(currentKey); setDraft(next); setArgumentsText(next.arguments.join("\n")); }
  const configuration = { ...draft, arguments: argumentsText.split(/\r?\n/u).filter(value => value.length > 0) };
  const changed = JSON.stringify(configuration) !== currentKey;
  const disabled = props.disabled || props.busy || pending || props.readOnly;
  const set = (patch: Partial<LocalProgramConfiguration>) => { setDraft(value => ({ ...value, ...patch })); setError(null); };
  const choose = async (field: "executable" | "modelPath", directory = false) => {
    if (disabled || inFlight.current) return;
    inFlight.current = true; setPending(true); setError(null);
    try { const path = await pickLocalProgramPath(directory, field === "executable" ? copy.chooseProgram : directory ? copy.chooseFolder : copy.chooseModel); if (mounted.current && path) set({ [field]: path }); }
    catch (failure) { if (mounted.current) setError(localProgramError(failure)); }
    finally { inFlight.current = false; if (mounted.current) setPending(false); }
  };
  const save = async () => {
    if (disabled || inFlight.current) return;
    inFlight.current = true; setPending(true); setError(null);
    const notify = beginToast();
    try { await props.onSaveProgram(configuration); notify(copy.saved); }
    catch (failure) { if (mounted.current) setError(localProgramError(failure)); }
    finally { inFlight.current = false; if (mounted.current) setPending(false); }
  };
  const id = props.inputId;
  return <>
    <section className="local-program service-stage" aria-label={copy.own}>
      <div className="local-program__heading"><h3>{copy.own}</h3><SettingsHelp text={copy.programHelp} label={I18N.settings.helpLabel} /></div>
      <label className="settings-field"><span>{copy.engine}<SettingsHelp text={draft.engine === "whisperCpp" ? copy.whisperHelp : copy.workerHelp} label={I18N.settings.helpLabel} /></span>
        <SettingsSelect label={copy.engine} value={draft.engine} disabled={disabled} onChange={engine => set({ engine: engine as LocalProgramEngine })} options={[{ value: "whisperCpp", label: "whisper.cpp" }, { value: "mimiStdio", label: copy.worker }]} />
      </label>
      <div className="local-program__path"><label className="settings-field" htmlFor={`${id}-program`}><span>{copy.executable}</span><ConfigInput id={`${id}-program`} value={draft.executable} placeholder={copy.chooseProgram} disabled={disabled} spellCheck={false} expandable onValueChange={executable => set({ executable })} /></label>
        <button type="button" className="settings-button settings-button--compact" disabled={disabled} onClick={() => void choose("executable")}>{copy.chooseProgram}</button>
      </div>
      <div className="local-program__path"><label className="settings-field" htmlFor={`${id}-model`}><span>{copy.model}</span><ConfigInput id={`${id}-model`} value={draft.modelPath} placeholder={draft.engine === "whisperCpp" ? copy.chooseModel : copy.chooseFolder} disabled={disabled} spellCheck={false} expandable onValueChange={modelPath => set({ modelPath })} /></label>
        <div className="local-program__pickers"><button type="button" className="settings-button settings-button--compact" disabled={disabled} onClick={() => void choose("modelPath")}>{copy.chooseModel}</button>{draft.engine === "mimiStdio" && <button type="button" className="settings-button settings-button--compact" disabled={disabled} onClick={() => void choose("modelPath", true)}>{copy.chooseFolder}</button>}</div>
      </div>
      <details className="local-program__arguments"><summary><Icon name="chevron-down" />{copy.advanced}<SettingsHelp text={copy.argumentsHelp} label={I18N.settings.helpLabel} /></summary><textarea aria-label={copy.advanced} rows={4} value={argumentsText} disabled={disabled} spellCheck={false} onChange={event => { setArgumentsText(event.target.value); setError(null); }} /></details>
      {error && <InlineFeedback tone="error">{error}</InlineFeedback>}
      <div className="local-program__actions"><button type="button" className="settings-button settings-button--primary settings-button--compact" disabled={disabled || !changed || !draft.executable.trim() || !draft.modelPath.trim()} onClick={() => void save()}>{copy.save}</button>
      <div className="local-program__check">{typeof props.connectionCheck === "function" ? props.connectionCheck(changed || !props.profile.localProgram ? null : undefined) : props.connectionCheck}{changed && <SettingsHelp text={copy.saveBeforeCheck} label={I18N.settings.helpLabel} />}</div></div>
    </section>
    <AlibabaCredentialEditor {...props} textOnly />
  </>;
}
