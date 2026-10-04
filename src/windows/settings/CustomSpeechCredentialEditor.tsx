import { useEffect, useRef, useState, type ComponentProps, type FormEvent } from "react";
import { ProviderIcon } from "../../components/ProviderIcon";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { credentialUnavailableHelp } from "../../lib/connectionDiagnostics";
import { buildCustomSpeechCredentials, customSpeechEndpointIsValid, emptyCredentialDraft, openAICompatibleModelIsValid } from "../../lib/providerCredentials";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";
import { SavedCredentialInput } from "./SavedCredentialInput";
import { useCredentialEditorState } from "./useCredentialEditorState";
import { ConfigInput } from "./ConfigInput";
import { SettingsHelp } from "./SettingsHelp";
import { CredentialStorageHelp } from "./CredentialStorageHelp";
import { InlineFeedback } from "./SettingsPrimitives";
import { DestructiveConfirmation } from "./DestructiveConfirmation";

/** Independent ASR and text credentials; neither form can write the other's key. */
export function CustomSpeechCredentialEditor(props: ComponentProps<typeof AlibabaCredentialEditor>) {
  const { profile, inputId, disabled, busy, feedback, onSave, onRequestDelete, onConfirmDelete, confirmingDelete, onCancelDelete, connectionCheck, readOnly = false } = props;
  const [draft, setDraft] = useState(emptyCredentialDraft);
  const [editing, setEditing] = useState(false);
  const [changedFields, setChangedFields] = useState<{ endpoint?: true; model?: true }>({});
  const [speechEpoch, setSpeechEpoch] = useState(0);
  const [translationEpoch, setTranslationEpoch] = useState(0);
  const [invalid, setInvalid] = useState<"endpoint" | "model" | null>(null);
  const saved = profile.speechCredentialState === "present";
  const editorState = useCredentialEditorState(profile.id, undefined, !readOnly && props.visible !== false && (!saved || editing), speechEpoch);
  const savedValues = editorState.state;
  const endpoint = changedFields.endpoint ? draft.endpoint : savedValues?.endpoint ?? "";
  const model = changedFields.model ? draft.model : savedValues?.model ?? "";
  const changedEndpoint = changedFields.endpoint && endpoint.trim() !== (savedValues?.endpoint ?? "");
  const changedModel = changedFields.model && model.trim() !== (savedValues?.model ?? "");
  const speechDraft = {
    apiKey: draft.apiKey,
    endpoint: changedEndpoint || !saved ? endpoint : "",
    model: changedModel || changedEndpoint || !saved ? model : "",
  };
  const credentials = saved && (editorState.loading || editorState.error) ? null : buildCustomSpeechCredentials(profile, speechDraft);
  const endpointRef = useRef<HTMLInputElement>(null);
  const modelRef = useRef<HTMLInputElement>(null);
  const feedbackRef = useRef<HTMLDivElement>(null);
  const speechId = `${inputId}-speech`;
  const noteId = `${inputId}-storage-note`;
  const helpId = `${speechId}-requirements`;
  const openAI = profile.provider === "customOpenAIASR";
  const requirements = [openAI ? I18N.settings.customSpeechRequirementsOpenAI : I18N.settings.customSpeechRequirementsDashScope, I18N.settings.customSpeechLanguages].join("\n");
  const discard = () => { setDraft(emptyCredentialDraft()); setChangedFields({}); setSpeechEpoch(current => current + 1); setEditing(false); setInvalid(null); };
  useEffect(() => {
    const ref = invalid === "endpoint" ? endpointRef : invalid === "model" ? modelRef : null;
    ref?.current?.focus({ preventScroll: true });
    ref?.current?.closest("label")?.scrollIntoView({ block: "center" });
  }, [invalid]);
  useEffect(() => {
    if (feedback?.tone === "error") {
      feedbackRef.current?.scrollIntoView({ block: "center" });
      feedbackRef.current?.focus({ preventScroll: true });
    }
  }, [feedback]);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!credentials || readOnly || disabled) return;
    if ((changedFields.endpoint || !saved) && !customSpeechEndpointIsValid(endpoint)) { setInvalid("endpoint"); return; }
    if ((changedFields.model || !saved) && !openAICompatibleModelIsValid(model)) { setInvalid("model"); return; }
    setInvalid(null);
    void onSave(credentials).then(result => { if (result) discard(); });
  };
  return <div className="credential-panel" aria-busy={busy}>
    <div className="service-credential-toolbar">
      <CredentialStorageHelp id={noteId} profile={profile} readOnly={readOnly} />
      {connectionCheck}
      {!readOnly && saved && !editing && <span className="credential-panel__saved-actions">
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={() => setEditing(true)}><Icon name="key" />{I18N.settings.editSpeechConfiguration}</button>
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled || confirmingDelete} onClick={onRequestDelete}><Icon name="trash" />{I18N.settings.deleteCredentials}</button>
      </span>}
    </div>
    {!readOnly && profile.credentialState === "unavailable" && !feedback && <p role="status" className="credential-unavailable">{credentialUnavailableHelp()}</p>}
    <section className="service-stage" aria-labelledby={`${speechId}-title`}>
      <header className="service-stage__heading"><div className="service-stage__name-help"><h3 id={`${speechId}-title`}>{I18N.settings.speechRecognition}</h3><SettingsHelp id={helpId} text={requirements} label={I18N.settings.helpLabel} /></div>
      </header>
      <div className="settings-field service-stage__selector"><span>{I18N.settings.serviceProvider}</span><span className="service-stage__provider"><ProviderIcon provider={profile.provider} size={32} />{openAI ? "OpenAI Realtime ASR" : "DashScope ASR"}</span></div>
      {!readOnly && (!saved || editing) && <form className="credential-form" onSubmit={submit}>
        {editorState.error && <InlineFeedback tone="error">{editorState.error}<button type="button" className="settings-link" disabled={disabled} onClick={() => setSpeechEpoch(current => current + 1)}>{I18N.settings.retryLoadingSettings}</button></InlineFeedback>}
        <div className="credential-form__fields">
          <label className="settings-field" htmlFor={`${speechId}-endpoint`}>
            <span>{I18N.settings.customSpeechEndpoint}</span>
            <ConfigInput ref={endpointRef} id={`${speechId}-endpoint`} type="text" autoComplete="off" spellCheck={false} disabled={disabled} required value={endpoint} placeholder={openAI ? "wss://api.openai.com/v1/realtime" : "wss://dashscope.aliyuncs.com/api-ws/v1/inference"} aria-describedby={invalid === "endpoint" ? `${speechId}-endpoint-error ${helpId}` : helpId} aria-invalid={invalid === "endpoint" || undefined} onValueChange={endpoint => { setChangedFields(current => ({ ...current, endpoint: true })); setDraft(current => ({ ...current, endpoint })); if (invalid === "endpoint" && customSpeechEndpointIsValid(endpoint)) setInvalid(null); }} />
            {invalid === "endpoint" && <span id={`${speechId}-endpoint-error`} role="alert" className="credential-unavailable">{I18N.settings.customSpeechEndpointInvalid}</span>}
          </label>
          <label className="settings-field" htmlFor={`${speechId}-model`}>
            <span>{I18N.settings.customSpeechModel}</span>
            <ConfigInput ref={modelRef} id={`${speechId}-model`} type="text" autoComplete="off" spellCheck={false} disabled={disabled} required value={model} placeholder={openAI ? "gpt-4o-mini-transcribe" : "qwen-audio-3.0-asr-flash-streaming"} aria-describedby={invalid === "model" ? `${speechId}-model-error ${helpId}` : helpId} aria-invalid={invalid === "model" || undefined} onValueChange={model => { setChangedFields(current => ({ ...current, model: true })); setDraft(current => ({ ...current, model })); if (invalid === "model" && openAICompatibleModelIsValid(model)) setInvalid(null); }} />
            {invalid === "model" && <span id={`${speechId}-model-error`} role="alert" className="credential-unavailable">{I18N.settings.customSpeechModelInvalid}</span>}
          </label>
          <div className="settings-field">
            <span className="service-stage__field-label"><label htmlFor={`${speechId}-key`}>{I18N.settings.apiKey}</label><SettingsHelp id={`${speechId}-address-key`} text={I18N.settings.customSpeechAddressKey} label={I18N.settings.helpLabel} /></span>
            <SavedCredentialInput key={`${profile.id}:${speechEpoch}`} id={`${speechId}-key`} profileId={profile.id} field="apiKey" label={I18N.settings.apiKey} hasSavedValue={(savedValues?.savedFields.includes("apiKey") ?? false) && !changedEndpoint} active={props.visible !== false && !busy && !confirmingDelete} autoComplete="new-password" spellCheck={false} disabled={disabled} required={!saved || !!changedEndpoint} value={draft.apiKey} placeholder={saved && !changedEndpoint ? I18N.settings.savedTranslationKeyPlaceholder : I18N.settings.apiKeyPlaceholder} aria-describedby={`${speechId}-address-key ${noteId}`} onValueChange={value => setDraft(current => ({ ...current, apiKey: value }))} />
          </div>
        </div>
        <span className="credential-form__actions">
          {saved && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={discard}>{I18N.settings.cancel}</button>}
          <button type="submit" className="settings-button settings-button--primary settings-button--compact" disabled={disabled || !credentials}><Icon name="key" />{I18N.settings.saveSpeechConfiguration}</button>
        </span>
      </form>}
    </section>
    <AlibabaCredentialEditor key={translationEpoch} {...props} inputId={`${inputId}-text`} storageNoteId={noteId} textOnly feedback={null} connectionCheck={undefined} />
    {feedback && <div ref={feedbackRef} tabIndex={-1}><InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback></div>}
    {!readOnly && confirmingDelete && <DestructiveConfirmation message={I18N.settings.deleteCredentialsConfirm} disabled={disabled} onCancel={onCancelDelete} onConfirm={() => { discard(); setTranslationEpoch(current => current + 1); void onConfirmDelete(); }} />}
  </div>;
}
