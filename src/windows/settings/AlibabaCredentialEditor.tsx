import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { ProviderIcon } from "../../components/ProviderIcon";
import { ConfigInput, type ConfigInputElement } from "./ConfigInput";
import { SettingsHelp } from "./SettingsHelp";
import { CredentialStorageHelp } from "./CredentialStorageHelp";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { credentialUnavailableHelp, diagnosticCopy } from "../../lib/connectionDiagnostics";
import { isChatCompletionsTranslation, textTranslationForProfile } from "../../lib/providerCapabilities";
import { CHATMOCK_DEFAULT_ENDPOINT, buildAlibabaTranslationCredentials, deepLXEndpointIsValid, emptyCredentialDraft, openAICompatibleModelIsValid } from "../../lib/providerCredentials";
import type { ProviderCredentialsInput, ServiceProfile, SourceLanguage, TextTranslation } from "../../lib/types";
import { DestructiveConfirmation } from "./DestructiveConfirmation";
import { InlineFeedback, SettingsSelect } from "./SettingsPrimitives";
import { TextTranslationName } from "./TextTranslationName";
import { textTranslationDisplayName } from "../../lib/textTranslationName";
import type { TextTranslationNameDraft } from "../../lib/types";
import { SavedCredentialInput } from "./SavedCredentialInput";
import { useCredentialEditorState } from "./useCredentialEditorState";

/** Alibaba provides recognition; independent text destinations use their own credentials. */
export function AlibabaCredentialEditor({ profile, inputId, disabled, busy, visible = true, feedback, onSave, onRequestDelete, onConfirmDelete, confirmingDelete, onCancelDelete, connectionCheck, textConnectionCheck, readOnly = false, textOnly = false, storageNoteId, onSaveTranslationName }: {
  onSaveTranslationName?: (route: TextTranslationNameDraft["route"], name: string) => Promise<unknown>;
  connectionCheck?: ReactNode | ((draft?: ProviderCredentialsInput | null, sourceLanguage?: SourceLanguage) => ReactNode);
  textConnectionCheck?: (draft?: ProviderCredentialsInput | null) => ReactNode;
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
  const [editedFields, setEditedFields] = useState({ endpoint: false, model: false });
  const [endpointInvalid, setEndpointInvalid] = useState(false);
  const [modelInvalid, setModelInvalid] = useState(false);
  const [revealEpoch, setRevealEpoch] = useState(0);
  const [clearTranslationToken, setClearTranslationToken] = useState(false);
  const recognitionState = useCredentialEditorState(profile.id, undefined, visible && !readOnly && !textOnly, revealEpoch);
  const translationState = useCredentialEditorState(profile.id, savedTranslation === "followService" ? undefined : savedTranslation, visible && !readOnly && savedTranslation !== "followService" && translation === savedTranslation, revealEpoch);
  const endpointValue = editedFields.endpoint ? draft.endpoint : translationState.state?.endpoint ?? draft.endpoint;
  const modelValue = editedFields.model ? draft.model : translationState.state?.model ?? draft.model;
  const endpointChanged = editedFields.endpoint && draft.endpoint.trim() !== (translationState.state?.endpoint ?? "");
  const modelChanged = editedFields.model && draft.model.trim() !== (translationState.state?.model ?? "");
  const endpointDraft = translation !== savedTranslation ? endpointValue : endpointChanged ? draft.endpoint : "";
  const modelDraft = translation !== savedTranslation ? modelValue : modelChanged ? draft.model : "";
  const hasTranslationKey = translationState.state?.savedFields.includes("token") ?? false;
  const endpointRef = useRef<ConfigInputElement>(null);
  const modelRef = useRef<ConfigInputElement>(null);
  const feedbackRef = useRef<HTMLDivElement>(null);
  const saved = (textOnly ? profile.textCredentialState : profile.credentialState) === "present" || (textOnly && savedTranslation === "followService");
  // Replacement drafts and saved-value previews must never carry across
  // routes, including a destination changed by another window.
  if (draftTranslation !== translation) {
    setDraftTranslation(translation);
    setDraft((current) => ({ ...current, endpoint: translation === "chatMock" && translation !== savedTranslation ? CHATMOCK_DEFAULT_ENDPOINT : "", token: "", model: "" }));
    setEditedFields({ endpoint: false, model: false });
    setEndpointInvalid(false);
    setModelInvalid(false);
    setClearTranslationToken(false);
  }
  const credentials = buildAlibabaTranslationCredentials(profile, { ...draft, endpoint: endpointDraft, model: modelDraft }, translation, clearTranslationToken);
  const dirty = !saved || editingKey || translation !== savedTranslation || endpointChanged || !!draft.token || modelChanged || clearTranslationToken;
  const compatible = isChatCompletionsTranslation(translation);
  const keepsSavedDestination = saved && translation === savedTranslation;
  const awaitingSavedDestination = keepsSavedDestination && (translationState.loading || !!translationState.error);
  const savedSpeech = (profile.speechCredentialState ?? profile.credentialState) === "present";
  const unavailableSpeech = (profile.speechCredentialState ?? profile.credentialState) === "unavailable";
  const keepsSavedText = (profile.textCredentialState ?? profile.credentialState) === "present" && translation === savedTranslation;
  const speechCheckDraft: ProviderCredentialsInput | null | undefined = readOnly || !draft.apiKey.trim()
    ? readOnly || savedSpeech || (unavailableSpeech && !draft.apiKey) ? undefined : null
    : { kind: "alibabaTranslation", apiKey: draft.apiKey.trim(), textTranslation: "followService", endpoint: "", token: "", model: "" };
  const textChanged = translation !== savedTranslation || endpointChanged || modelChanged || !!draft.token.trim() || clearTranslationToken || (translation === "followService" && !!draft.apiKey.trim());
  let textCheckDraft: ProviderCredentialsInput | null | undefined;
  if (!readOnly && textChanged) {
    const endpointRequired = translation === "deepLX" || compatible;
    const invalidEndpoint = endpointRequired && (editedFields.endpoint || !keepsSavedText) && !deepLXEndpointIsValid(endpointValue);
    const invalidModel = compatible && (editedFields.model || !keepsSavedText) && !openAICompatibleModelIsValid(modelValue);
    const missingToken = translation === "deepL" && !draft.token.trim() && !keepsSavedText;
    const completeExplicitText = translation === "deepL" ? !!draft.token.trim()
      : endpointRequired && deepLXEndpointIsValid(draft.endpoint) && (!compatible || openAICompatibleModelIsValid(draft.model)) && (!!draft.token.trim() || (compatible && clearTranslationToken));
    const unavailableSavedText = keepsSavedText && (translationState.loading || !!translationState.error) && !completeExplicitText;
    const unavailableBuiltIn = translation === "followService" && (textOnly || (!draft.apiKey.trim() && !savedSpeech));
    // A text-only probe must not need or submit a recognition key. Include
    // known nonsecret configuration so legacy combined records can be checked
    // without reading their speech slot; native normalization preserves auth
    // when the displayed address still identifies the same destination.
    textCheckDraft = invalidEndpoint || invalidModel || missingToken || unavailableSavedText || unavailableBuiltIn ? null : {
      kind: "alibabaTranslation",
      apiKey: translation === "followService" ? draft.apiKey.trim() : "",
      textTranslation: translation,
      endpoint: endpointRequired ? endpointValue.trim() : "",
      model: compatible ? modelValue.trim() : "",
      token: translation === "followService" || clearTranslationToken ? "" : draft.token.trim(),
      ...(compatible && clearTranslationToken ? { clearToken: true } : {}),
    };
  }
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
    setEditedFields({ endpoint: false, model: false });
    setEndpointInvalid(false);
    setModelInvalid(false);
    setClearTranslationToken(false);
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!credentials || disabled || readOnly || awaitingSavedDestination) return;
    if ((translation === "deepLX" || compatible) && (editedFields.endpoint || !keepsSavedDestination) && !deepLXEndpointIsValid(endpointValue)) {
      setEndpointInvalid(true);
      return;
    }
    setEndpointInvalid(false);
    if (compatible && (editedFields.model || !keepsSavedDestination) && !openAICompatibleModelIsValid(modelValue)) {
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
  ].map(option => option.value === "followService" ? option : { ...option, label: textTranslationDisplayName(profile, option.value as TextTranslation) });
  const selectedTranslation = translationOptions.find(option => option.value === translation)!;

  const stages = <>
    {!textOnly && <section className="service-stage" aria-labelledby={`${inputId}-recognition-title`}>
      <header className="service-stage__heading">
        <h3 id={`${inputId}-recognition-title`}>{I18N.settings.speechRecognition}</h3>
      </header>
      <div className="settings-field service-stage__selector">
        <span>{I18N.settings.serviceProvider}</span>
        <span className="service-stage__provider"><ProviderIcon provider="alibabaCloud" size={32} />Alibaba Cloud</span>
      </div>
      {!readOnly && (!saved || editingKey) && <div className="settings-field">
        <label htmlFor={`${inputId}-apiKey`}>{I18N.settings.apiKey}</label>
        <SavedCredentialInput key={`${profile.id}:${revealEpoch}`} profileId={profile.id} field={profile.provider === "deepLX" ? "asrApiKey" : "apiKey"} label={I18N.settings.apiKey} hasSavedValue={recognitionState.state?.savedFields.includes(profile.provider === "deepLX" ? "asrApiKey" : "apiKey") ?? false} active={visible && !busy && !confirmingDelete} id={`${inputId}-apiKey`} type="password" autoComplete="new-password" spellCheck={false} disabled={disabled} value={draft.apiKey} placeholder={I18N.settings.apiKeyPlaceholder} aria-describedby={noteId} onValueChange={(value) => { setEditingKey(true); setDraft((current) => ({ ...current, apiKey: value })); }} />
      </div>}
    </section>}
    <section className="service-stage service-stage--translation" aria-labelledby={`${inputId}-translation-title`}>
      <header className="service-stage__heading">
        <div className="service-stage__name-help"><h3 id={`${inputId}-translation-title`}>{I18N.settings.textTranslationLabel}</h3><SettingsHelp text={translationHelp} label={I18N.settings.helpLabel} /></div>
        <div className="service-stage__actions">
          {!(textOnly && translation === "followService") && textConnectionCheck?.(textCheckDraft)}
        </div>
      </header>
      <div className="settings-field service-stage__selector">
        <span>{I18N.settings.serviceProvider}</span>
        {readOnly ? <span className="service-stage__provider">{selectedTranslation.icon}{selectedTranslation.label}</span>
          : <SettingsSelect label={I18N.settings.textTranslationLabel} disabled={disabled} value={translation} options={translationOptions}
            onChange={(value) => { setTranslationDraft(value as TextTranslation); setEndpointInvalid(false); }} />}
      </div>
      {!readOnly && translation !== "followService" && onSaveTranslationName && <TextTranslationName
        key={`${profile.id}:${translation}`} profile={profile} route={translation} inputId={`${inputId}-name`}
        disabled={disabled} onSave={onSaveTranslationName} />}
      {readOnly && <div className="service-stage__restriction"><InlineFeedback tone="info" icon="lock">{diagnosticCopy().localDevTranslationLocked}</InlineFeedback><SettingsHelp text={diagnosticCopy().localDevTranslationHelp} label={I18N.settings.helpLabel} /></div>}
      {!readOnly && translation !== "followService" && <div className="credential-form__fields">
        {(translation === "deepLX" || compatible) && <label className="settings-field" htmlFor={endpointId}>
          <span>{compatible ? I18N.settings.openAICompatibleEndpoint : I18N.settings.deepLXEndpoint}</span>
          <ConfigInput expandable key={translation} ref={endpointRef} id={endpointId} type="text" autoComplete="off" spellCheck={false} disabled={disabled} required value={endpointValue} placeholder={I18N.settings.serviceAddressPlaceholder} aria-invalid={endpointInvalid || undefined} aria-describedby={endpointInvalid ? `${endpointId}-error ${noteId}` : noteId} onValueChange={(value) => { setEditedFields((current) => ({ ...current, endpoint: true })); setDraft((current) => ({ ...current, endpoint: value })); if (endpointInvalid) setEndpointInvalid(!deepLXEndpointIsValid(value)); }} />
          {endpointInvalid && <span id={`${endpointId}-error`} role="alert" className="credential-unavailable">{I18N.settings.deepLXEndpointInvalid}</span>}
        </label>}
        {compatible && <label className="settings-field" htmlFor={modelId}>
          <span>{I18N.settings.openAICompatibleModel}</span>
          <ConfigInput expandable key={translation} ref={modelRef} id={modelId} type="text" autoComplete="off" spellCheck={false} disabled={disabled} required value={modelValue} placeholder={translation === "chatMock" ? I18N.settings.chatMockModelPlaceholder : I18N.settings.modelNamePlaceholder} aria-invalid={modelInvalid || undefined} aria-describedby={modelInvalid ? `${modelId}-error ${noteId}` : noteId} onValueChange={(value) => { setEditedFields((current) => ({ ...current, model: true })); setDraft((current) => ({ ...current, model: value })); if (modelInvalid) setModelInvalid(!openAICompatibleModelIsValid(value)); }} />
          {modelInvalid && <span id={`${modelId}-error`} role="alert" className="credential-unavailable">{I18N.settings.openAICompatibleModelInvalid}</span>}
        </label>}
        <div className="settings-field">
          <span className="service-stage__field-label"><label htmlFor={`${inputId}-token`}>{destinationKeyLabel}</label>{compatible && <SettingsHelp id={`${inputId}-compatible-required`} text={keepsSavedDestination ? I18N.settings.openAICompatibleAddressKey : I18N.settings.openAICompatibleRequired} label={I18N.settings.helpLabel} />}</span>
          <SavedCredentialInput key={`${profile.id}:${translation}:${revealEpoch}`} profileId={profile.id} field="token" textTranslation={translation} label={destinationKeyLabel} hasSavedValue={hasTranslationKey && !clearTranslationToken && !endpointChanged} active={visible && !busy && !confirmingDelete} id={`${inputId}-token`} type="password" autoComplete="new-password" spellCheck={false} disabled={disabled || clearTranslationToken} value={draft.token} placeholder={compatible ? clearTranslationToken ? I18N.settings.noTranslationKeyPlaceholder : hasTranslationKey && !endpointChanged ? I18N.settings.savedTranslationKeyPlaceholder : I18N.settings.optionalTranslationKeyPlaceholder : translation === "deepL" ? hasTranslationKey ? I18N.settings.savedTranslationKeyPlaceholder : I18N.settings.apiKeyPlaceholder : undefined} aria-describedby={compatible ? `${inputId}-compatible-required ${noteId}` : noteId} onValueChange={(value) => setDraft((current) => ({ ...current, token: value }))} />
        </div>
        {compatible && keepsSavedDestination && hasTranslationKey && <span className="credential-form__actions">
          <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} aria-pressed={clearTranslationToken} onClick={() => { setClearTranslationToken((current) => !current); setDraft((current) => ({ ...current, token: "" })); }}>
            <Icon name="key" />{clearTranslationToken ? I18N.settings.cancelTranslationKeyRemoval : I18N.settings.removeTranslationApiKey}
          </button>
        </span>}
      </div>}
    </section>
  </>;

  return <div className="credential-panel" aria-busy={busy}>
    {!textOnly && <div className="service-credential-toolbar">
      <CredentialStorageHelp id={noteId} profile={profile} readOnly={readOnly} />
      {typeof connectionCheck === "function" ? connectionCheck(speechCheckDraft) : connectionCheck}
      {!readOnly && saved && !dirty && <span className="credential-panel__saved-actions">
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={() => setEditingKey(true)}><Icon name="key" />{I18N.settings.replaceCredentials}</button>
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled || confirmingDelete} onClick={onRequestDelete}><Icon name="trash" />{I18N.settings.deleteCredentials}</button>
      </span>}
    </div>}
    {!textOnly && !readOnly && profile.credentialState === "unavailable" && (feedback?.tone !== "error" || feedback.message === I18N.settings.profileActionFailed) && <p role="status" className="credential-unavailable">{credentialUnavailableHelp()}</p>}
    {readOnly ? <div className="service-stages service-stages--readonly">{stages}</div> : <form className="credential-form service-stages" onSubmit={submit}>
      {stages}
      {(recognitionState.error || translationState.error) && <InlineFeedback tone="error">{recognitionState.error ?? translationState.error}<button type="button" className="settings-link" disabled={disabled} onClick={() => setRevealEpoch(current => current + 1)}>{I18N.settings.retryLoadingSettings}</button></InlineFeedback>}
      {feedback && <div ref={feedbackRef} tabIndex={-1}><InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback></div>}
      {dirty && <span className="credential-form__actions">
        {saved && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={discard}>{I18N.settings.cancel}</button>}
        <button type="submit" className="settings-button settings-button--primary settings-button--compact" disabled={disabled || awaitingSavedDestination || !credentials || (!textOnly && editingKey && !draft.apiKey.trim())}><Icon name="key" />{textOnly ? I18N.settings.saveTranslationConfiguration : saved ? I18N.settings.replaceCredentials : I18N.settings.saveAndUse}</button>
      </span>}
    </form>}
    {!textOnly && !readOnly && confirmingDelete && <DestructiveConfirmation message={I18N.settings.deleteCredentialsConfirm} disabled={disabled} onCancel={onCancelDelete} onConfirm={() => { discard(); void onConfirmDelete(); }} />}
  </div>;
}
