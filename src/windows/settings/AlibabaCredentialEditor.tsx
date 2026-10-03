import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { ProviderIcon } from "../../components/ProviderIcon";
import { ConfigInput } from "./ConfigInput";
import { SettingsHelp } from "./SettingsHelp";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { credentialUnavailableHelp, diagnosticCopy } from "../../lib/connectionDiagnostics";
import { isChatCompletionsTranslation, textTranslationForProfile } from "../../lib/providerCapabilities";
import { CHATMOCK_DEFAULT_ENDPOINT, buildAlibabaTranslationCredentials, deepLXEndpointIsValid, emptyCredentialDraft, openAICompatibleModelIsValid } from "../../lib/providerCredentials";
import type { ProviderCredentialsInput, ServiceProfile, TextTranslation } from "../../lib/types";
import { DestructiveConfirmation } from "./DestructiveConfirmation";
import { InlineFeedback, SettingsSelect } from "./SettingsPrimitives";
import { StoredCredentialReveal } from "./StoredCredentialReveal";

/** Alibaba provides recognition; independent text destinations use their own credentials. */
export function AlibabaCredentialEditor({ profile, inputId, disabled, busy, visible = true, feedback, onSave, onRequestDelete, onConfirmDelete, confirmingDelete, onCancelDelete, connectionCheck, textConnectionCheck, readOnly = false, textOnly = false, storageNoteId }: {
  connectionCheck?: ReactNode;
  textConnectionCheck?: (requiresSave: boolean) => ReactNode;
  readOnly?: boolean;
  textOnly?: boolean;
  storageNoteId?: string;
  profile: ServiceProfile;
  inputId: string;
  disabled: boolean;
  busy: boolean;
  visible?: boolean;
  feedback: { tone: "success" | "error" | "info"; message: string } | null;
  onSave: (credentials: ProviderCredentialsInput) => Promise<unknown>;
  onRequestDelete: () => void;
  onConfirmDelete: () => Promise<unknown>;
  confirmingDelete: boolean;
  onCancelDelete: () => void;
}) {
  const savedTranslation = textTranslationForProfile(profile);
  const [translationDraft, setTranslationDraft] = useState<TextTranslation | null>(null);
  const translation = translationDraft ?? savedTranslation;
  const [draft, setDraft] = useState(emptyCredentialDraft);
  const [draftTranslation, setDraftTranslation] = useState(translation);
  const [editingKey, setEditingKey] = useState(false);
  const [endpointInvalid, setEndpointInvalid] = useState(false);
  const [modelInvalid, setModelInvalid] = useState(false);
  const [revealEpoch, setRevealEpoch] = useState(0);
  const [clearTranslationToken, setClearTranslationToken] = useState(false);
  const endpointRef = useRef<HTMLInputElement>(null);
  const modelRef = useRef<HTMLInputElement>(null);
  const feedbackRef = useRef<HTMLDivElement>(null);
  const saved = (textOnly ? profile.textCredentialState : profile.credentialState) === "present" || (textOnly && savedTranslation === "followService");
  // Replacement drafts and saved-value previews must never carry across
  // routes, including a destination changed by another window.
  if (draftTranslation !== translation) {
    setDraftTranslation(translation);
    setDraft((current) => ({ ...current, endpoint: translation === "chatMock" && translation !== savedTranslation ? CHATMOCK_DEFAULT_ENDPOINT : "", token: "", model: "" }));
    setEndpointInvalid(false);
    setModelInvalid(false);
    setClearTranslationToken(false);
  }
  const credentials = buildAlibabaTranslationCredentials(profile, draft, translation, clearTranslationToken);
  const dirty = !saved || editingKey || translation !== savedTranslation || !!draft.endpoint || !!draft.token || !!draft.model || clearTranslationToken;
  const compatible = isChatCompletionsTranslation(translation);
  const keepsSavedDestination = saved && translation === savedTranslation;
  const destinationKeyLabel = translation === "deepL" ? I18N.settings.deepLApiKey : compatible ? I18N.settings.openAICompatibleApiKey : I18N.settings.deepLXToken;
  const endpointId = `${inputId}-endpoint`;
  const modelId = `${inputId}-model`;
  const noteId = storageNoteId ?? `${inputId}-storage-note`;

  useEffect(() => {
    if (endpointInvalid) {
      endpointRef.current?.focus({ preventScroll: true });
      endpointRef.current?.closest("label")?.scrollIntoView({ block: "center" });
    }
  }, [endpointInvalid]);
  useEffect(() => {
    if (modelInvalid) {
      modelRef.current?.focus({ preventScroll: true });
      modelRef.current?.closest("label")?.scrollIntoView({ block: "center" });
    }
  }, [modelInvalid]);
  useEffect(() => {
    if (feedback?.tone === "error") {
      feedbackRef.current?.scrollIntoView({ block: "center" });
      feedbackRef.current?.focus({ preventScroll: true });
    }
  }, [feedback]);

  const discard = () => {
    setRevealEpoch((current) => current + 1);
    setDraft(emptyCredentialDraft());
    setTranslationDraft(null);
    setEditingKey(false);
    setEndpointInvalid(false);
    setModelInvalid(false);
    setClearTranslationToken(false);
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!credentials) return;
    if ((translation === "deepLX" || compatible) && draft.endpoint && !deepLXEndpointIsValid(draft.endpoint)) {
      setEndpointInvalid(true);
      return;
    }
    setEndpointInvalid(false);
    if (compatible && draft.model && !openAICompatibleModelIsValid(draft.model)) {
      setModelInvalid(true);
      return;
    }
    setModelInvalid(false);
    if (!saved) setEditingKey(true);
    void onSave(credentials).then((result) => { if (result) discard(); });
  };

  const translationHelp = translation === "chatMock" ? I18N.settings.chatMockSetup : textOnly ? [I18N.settings.customSpeechTranslationHelp, ...(compatible ? [I18N.settings.openAICompatibleRequirements, I18N.settings.openAICompatibleLanguages] : [])].join("\n") : translation === "followService" ? I18N.settings.textTranslationDefault
    : compatible ? [I18N.settings.openAICompatibleChain, I18N.settings.openAICompatibleRequirements, I18N.settings.openAICompatibleLanguages].join("\n")
    : translation === "deepL" ? I18N.settings.deepLChain : I18N.settings.deepLXChain;
  const translationOptions = [
    { value: "followService", label: textOnly ? I18N.settings.customSpeechNoTranslation : I18N.settings.textTranslationFollow, icon: textOnly ? <Icon name="captions-bubble" /> : <ProviderIcon provider="alibabaCloud" size={32} /> },
    { value: "deepL", label: "DeepL", icon: <ProviderIcon provider="deepL" size={32} /> },
    { value: "deepLX", label: I18N.settings.textTranslationCustom, icon: <ProviderIcon provider="deepLX" size={32} /> },
    { value: "chatMock", label: "ChatMock", icon: <ProviderIcon provider="chatMock" size={32} /> },
    { value: "openAICompatible", label: I18N.settings.textTranslationOpenAICompatible, icon: <ProviderIcon provider="openAICompatible" size={32} /> },
  ];
  const selectedTranslation = translationOptions.find(option => option.value === translation)!;

  const stages = <>
    {!textOnly && <section className="service-stage" aria-labelledby={`${inputId}-recognition-title`}>
      <header className="service-stage__heading">
        <h3 id={`${inputId}-recognition-title`}>{I18N.settings.speechRecognition}</h3>
        {!readOnly && saved && !editingKey && visible && !busy && !confirmingDelete && <StoredCredentialReveal key={`${profile.id}:${revealEpoch}`} profileId={profile.id} field={profile.provider === "deepLX" ? "asrApiKey" : "apiKey"} label={I18N.settings.apiKey} disabled={disabled} />}
      </header>
      <div className="settings-field service-stage__selector">
        <span>{I18N.settings.serviceProvider}</span>
        <span className="service-stage__provider"><ProviderIcon provider="alibabaCloud" size={32} />Alibaba Cloud</span>
      </div>
      {!readOnly && (!saved || editingKey) && <div className="settings-field">
        <label htmlFor={`${inputId}-apiKey`}>{I18N.settings.apiKey}</label>
        <ConfigInput id={`${inputId}-apiKey`} type="password" autoComplete="new-password" spellCheck={false} disabled={disabled} value={draft.apiKey} placeholder={I18N.settings.apiKeyPlaceholder} aria-describedby={noteId} onValueChange={(value) => { setEditingKey(true); setDraft((current) => ({ ...current, apiKey: value })); }} />
        {saved && visible && !busy && !confirmingDelete && <StoredCredentialReveal key={`${profile.id}:${revealEpoch}`} profileId={profile.id} field={profile.provider === "deepLX" ? "asrApiKey" : "apiKey"} label={I18N.settings.apiKey} disabled={disabled} />}
      </div>}
    </section>}
    <section className="service-stage service-stage--translation" aria-labelledby={`${inputId}-translation-title`}>
      <header className="service-stage__heading">
        <h3 id={`${inputId}-translation-title`}>{I18N.settings.textTranslationLabel}</h3>
        <div className="service-stage__actions">
          {textConnectionCheck?.(translation !== savedTranslation || !!draft.endpoint.trim() || !!draft.token.trim() || !!draft.model.trim() || clearTranslationToken || (!textOnly && translation === "followService" && !!draft.apiKey.trim()))}
          <SettingsHelp text={translationHelp} label={I18N.settings.helpLabel} />
        </div>
      </header>
      <div className="settings-field service-stage__selector">
        <span>{I18N.settings.serviceProvider}</span>
        {readOnly ? <span className="service-stage__provider">{selectedTranslation.icon}{selectedTranslation.label}</span>
          : <SettingsSelect label={I18N.settings.textTranslationLabel} disabled={disabled} value={translation} options={translationOptions}
            onChange={(value) => { setTranslationDraft(value as TextTranslation); setEndpointInvalid(false); }} />}
      </div>
      {readOnly && <div className="service-stage__restriction"><InlineFeedback tone="info" icon="lock">{diagnosticCopy().localDevTranslationLocked}</InlineFeedback><SettingsHelp text={diagnosticCopy().localDevTranslationHelp} label={I18N.settings.helpLabel} /></div>}
      {!readOnly && translation !== "followService" && <div className="credential-form__fields">
        {(translation === "deepLX" || compatible) && <label className="settings-field" htmlFor={endpointId}>
          <span>{compatible ? I18N.settings.openAICompatibleEndpoint : I18N.settings.deepLXEndpoint}</span>
          <ConfigInput key={translation} ref={endpointRef} id={endpointId} type="text" autoComplete="off" spellCheck={false} disabled={disabled} required={!keepsSavedDestination} value={draft.endpoint} placeholder={keepsSavedDestination ? I18N.settings.savedServiceAddressPlaceholder : translation === "chatMock" ? CHATMOCK_DEFAULT_ENDPOINT : compatible ? "https://dashscope.aliyuncs.com/compatible-mode/v1" : "https://example.com/translate"} aria-invalid={endpointInvalid || undefined} aria-describedby={endpointInvalid ? `${endpointId}-error ${noteId}` : noteId} onValueChange={(value) => { setDraft((current) => ({ ...current, endpoint: value })); if (endpointInvalid) setEndpointInvalid(!deepLXEndpointIsValid(value)); }} />
          {endpointInvalid && <span id={`${endpointId}-error`} role="alert" className="credential-unavailable">{I18N.settings.deepLXEndpointInvalid}</span>}
        </label>}
        {compatible && <label className="settings-field" htmlFor={modelId}>
          <span>{I18N.settings.openAICompatibleModel}</span>
          <ConfigInput key={translation} ref={modelRef} id={modelId} type="text" autoComplete="off" spellCheck={false} disabled={disabled} required={!keepsSavedDestination} value={draft.model} placeholder={keepsSavedDestination ? I18N.settings.savedTranslationModelPlaceholder : translation === "chatMock" ? I18N.settings.chatMockModelPlaceholder : "qwen-turbo"} aria-invalid={modelInvalid || undefined} aria-describedby={modelInvalid ? `${modelId}-error ${noteId}` : noteId} onValueChange={(value) => { setDraft((current) => ({ ...current, model: value })); if (modelInvalid) setModelInvalid(!openAICompatibleModelIsValid(value)); }} />
          {modelInvalid && <span id={`${modelId}-error`} role="alert" className="credential-unavailable">{I18N.settings.openAICompatibleModelInvalid}</span>}
        </label>}
        <div className="settings-field">
          <span className="service-stage__field-label"><label htmlFor={`${inputId}-token`}>{destinationKeyLabel}</label>{compatible && <SettingsHelp id={`${inputId}-compatible-required`} text={keepsSavedDestination ? I18N.settings.openAICompatibleAddressKey : I18N.settings.openAICompatibleRequired} label={I18N.settings.helpLabel} />}</span>
          <ConfigInput key={translation} id={`${inputId}-token`} type="password" autoComplete="new-password" spellCheck={false} disabled={disabled || clearTranslationToken} value={draft.token} placeholder={compatible ? clearTranslationToken ? I18N.settings.noTranslationKeyPlaceholder : keepsSavedDestination && !draft.endpoint.trim() ? I18N.settings.savedTranslationKeyPlaceholder : I18N.settings.optionalTranslationKeyPlaceholder : translation === "deepL" ? keepsSavedDestination ? I18N.settings.savedTranslationKeyPlaceholder : I18N.settings.apiKeyPlaceholder : undefined} aria-describedby={compatible ? `${inputId}-compatible-required ${noteId}` : noteId} onValueChange={(value) => setDraft((current) => ({ ...current, token: value }))} />
          {saved && savedTranslation === translation && visible && !busy && !confirmingDelete && <StoredCredentialReveal key={`${profile.id}:${translation}:${revealEpoch}`} profileId={profile.id} field="token" textTranslation={translation} label={destinationKeyLabel} disabled={disabled} />}
        </div>
        {compatible && keepsSavedDestination && <span className="credential-form__actions">
          <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} aria-pressed={clearTranslationToken} onClick={() => { setClearTranslationToken((current) => !current); setDraft((current) => ({ ...current, token: "" })); }}>
            <Icon name="key" />{clearTranslationToken ? I18N.settings.cancelTranslationKeyRemoval : I18N.settings.removeTranslationApiKey}
          </button>
        </span>}
      </div>}
    </section>
  </>;

  return <div className="credential-panel" aria-busy={busy}>
    {!textOnly && <div className="service-credential-toolbar">
      {connectionCheck}
      {!readOnly && saved && !dirty && <span className="credential-panel__saved-actions">
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={() => setEditingKey(true)}><Icon name="key" />{I18N.settings.replaceCredentials}</button>
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled || confirmingDelete} onClick={onRequestDelete}><Icon name="trash" />{I18N.settings.deleteCredentials}</button>
      </span>}
      <SettingsHelp id={noteId} text={readOnly ? profile.credentialState === "unavailable" ? diagnosticCopy().localDevUnavailable : diagnosticCopy().localDevReadOnly : I18N.settings.credentialNote} label={I18N.settings.helpLabel} icon="shield-check" />
    </div>}
    {!textOnly && !readOnly && profile.credentialState === "unavailable" && (feedback?.tone !== "error" || feedback.message === I18N.settings.profileActionFailed) && <p role="status" className="credential-unavailable">{credentialUnavailableHelp()}</p>}
    {readOnly ? <div className="service-stages service-stages--readonly">{stages}</div> : <form className="credential-form service-stages" onSubmit={submit}>
      {stages}
      {feedback && <div ref={feedbackRef} tabIndex={-1}><InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback></div>}
      {dirty && <span className="credential-form__actions">
        {saved && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={discard}>{I18N.settings.cancel}</button>}
        <button type="submit" className="settings-button settings-button--primary settings-button--compact" disabled={disabled || !credentials || (!textOnly && editingKey && !draft.apiKey.trim())}><Icon name="key" />{textOnly ? I18N.settings.saveTranslationConfiguration : saved ? I18N.settings.replaceCredentials : I18N.settings.saveAndUse}</button>
      </span>}
    </form>}
    {!textOnly && !readOnly && confirmingDelete && <DestructiveConfirmation message={I18N.settings.deleteCredentialsConfirm} disabled={disabled} onCancel={onCancelDelete} onConfirm={() => { discard(); void onConfirmDelete(); }} />}
  </div>;
}
