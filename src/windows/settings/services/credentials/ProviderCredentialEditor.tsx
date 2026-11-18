import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Icon } from "../../../../components/Icon";
import { I18N } from "../../../../lib/i18n";
import { diagnosticCopy } from "../../../../lib/providers/connectionDiagnostics";
import { SettingsConfirmation } from "../../controls/SettingsConfirmation";
import { isIndependentAsrProfile, supportsSpeechTranslationMode } from "../../../../lib/providers/providerCapabilities";
import {
  buildProviderCredentials,
  buildProviderProbeCredentials,
  deepLXEndpointIsValid,
  credentialEditorStateAfterDeleteRequest,
  credentialFieldsForProvider,
  emptyCredentialDraft,
  type CredentialDraft,
  type CredentialFieldName,
} from "../../../../lib/providers/providerCredentials";
import type { StoredCredentialField } from "../../../../lib/ipc";
import type {
  ProviderCredentialsInput,
  ServiceProfile,
  ServiceProvider,
  SourceLanguage,
  TargetLanguage,
  TextTranslationNameDraft
} from "../../../../lib/types";
import { ConfigInput, type ConfigInputElement } from "../../controls/ConfigInput";
import { SettingsHelp } from "../../feedback/SettingsHelp";
import { InlineFeedback } from "../../controls/SettingsPrimitives";
import { CredentialStorageHelp } from "./CredentialStorageHelp";
import { SavedCredentialInput } from "./SavedCredentialInput";
import { ServiceSetupLink } from "../providers/ServiceSetupLink";
import { TextTranslationEditor } from "./SpeechAndTextCredentialEditor";
import { TencentSetupHelp } from "../providers/TencentSetupHelp";
import { useCredentialEditorState } from "./useCredentialEditorState";

/** Owns write-only credential drafts, saved metadata, validation and plaintext
 * disposal. Profile selection and persistence remain with the catalog page. */
export function ProviderCredentialEditor(
  {
    profile,
    inputId,
    disabled,
    busy,
    visible,
    feedback,
    onSave,
    onRequestDelete,
    onConfirmDelete,
    confirmingDelete,
    onCancelDelete,
    connectionCheck,
    textConnectionCheck,
    speechModeSettings,
    onSaveTranslationName,
    sourceLanguage = "auto",
    targetLanguage = "original",
    requiresStop = false,
  }: {
    sourceLanguage?: SourceLanguage;
    targetLanguage?: TargetLanguage;
    requiresStop?: boolean;
    speechModeSettings?: ReactNode;
    onSaveTranslationName?: (route: TextTranslationNameDraft["route"], name: string) => Promise<unknown>;
    connectionCheck?: ReactNode | ((draft?: ProviderCredentialsInput | null) => ReactNode);
    textConnectionCheck?: (draft?: ProviderCredentialsInput | null) => ReactNode;
    profile: ServiceProfile;
    inputId: string;
    disabled: boolean;
    busy: boolean;
    visible: boolean;
    feedback: { tone: "success" | "error" | "info"; message: string } | null;
    onSave: (credentials: ProviderCredentialsInput) => Promise<unknown>;
    onRequestDelete: () => void;
    onConfirmDelete: () => Promise<unknown>;
    confirmingDelete: boolean;
    onCancelDelete: () => void;
  }
) {
  const [draft, setDraft] = useState<CredentialDraft>(emptyCredentialDraft);
  const [editingSavedCredential, setEditingSavedCredential] = useState(false);
  const [changedFields, setChangedFields] = useState<Partial<Record<CredentialFieldName, true>>>({});
  const [editorEpoch, setEditorEpoch] = useState(0);
  const editorState = useCredentialEditorState(profile.id, undefined, visible, editorEpoch);
  const savedValues = editorState.state;
  const displayedDraft = { ...draft };
  for(const field of ["endpoint", "model", "deployment", "transcriptionDeployment", "appId"] as const) {
    if(!changedFields[field]) displayedDraft[field] = savedValues?.[field] ?? "";
  }
  const [endpointInvalid, setEndpointInvalid] = useState(false);
  const endpointRef = useRef<ConfigInputElement>(null);
  const feedbackRef = useRef<HTMLDivElement>(null);
  useEffect(
    () => {
      if(endpointInvalid) {
        endpointRef.current?.focus({ preventScroll: true });
        endpointRef.current?.closest("label")?.scrollIntoView({ block: "center" });
      }
    },
    [endpointInvalid]
  );
  useEffect(
    () => {
      if(feedback?.tone === "error") {
        feedbackRef.current?.scrollIntoView({ block: "center" });
        feedbackRef.current?.focus({ preventScroll: true });
      }
    },
    [feedback]
  );
  const saveFeedback = feedback && (
    <div ref={feedbackRef} tabIndex={-1}>
      <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>
    </div>
  );
  const noteId = `${inputId}-storage-note`;
  const speechState = profile.speechCredentialState ?? profile.credentialState;
  // Azure saves share native endpoint-bound key resolution with checks. Only
  // loaded saved-field flags may authorize an unchanged, write-only key.
  const credentials = speechState === "present" && (editorState.loading || editorState.error)
    ? null
    : profile.provider === "azureOpenAIRealtime" && !editorState.loading && !editorState.error
      ? buildProviderProbeCredentials(
        profile.provider,
        displayedDraft,
        savedValues?.savedFields,
        profile.speechTranslationMode,
      )
      : buildProviderCredentials(profile.provider, displayedDraft, profile.speechTranslationMode);

  const hasCredentialChanges = credentialFieldsForProvider(profile.provider, profile.speechTranslationMode).some(
    field => {
      if(!changedFields[field]) return false;
      if(credentialFieldCopy(field, profile.provider).secret) return !!draft[field].trim();
      return draft[field].trim() !== (savedValues?.[field as keyof typeof savedValues] ?? "");
    }
  );
  // A complete explicit replacement can be checked without saved metadata.
  // Partial drafts may reuse saved fields only after that metadata loaded.
  const probeCredentials = buildProviderCredentials(profile.provider, draft, profile.speechTranslationMode)
    ?? (editorState.loading || editorState.error
      ? null
      : buildProviderProbeCredentials(
        profile.provider,
        displayedDraft,
        savedValues?.savedFields,
        profile.speechTranslationMode,
      ));
  const check = typeof connectionCheck === "function"
    ? connectionCheck(!hasCredentialChanges ? undefined : probeCredentials)
    : connectionCheck;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if(!credentials || disabled) return;
    if(profile.provider === "deepLX" && !deepLXEndpointIsValid(displayedDraft.endpoint)) {
      setEndpointInvalid(true);
      endpointRef.current?.focus();
      return;
    }
    setEndpointInvalid(false);
    setEditingSavedCredential(true);
    // Reading a saved value does not create a replacement draft. Keep only
    // the user's edits on failure; discard after a successful save/use.
    void onSave(credentials).then(
      (saved) => {
        if(saved) {
          setDraft(emptyCredentialDraft());
          setChangedFields({});
          setEditorEpoch(current => current + 1);
          setEditingSavedCredential(false);
        }
      }
    );
  };

  const handleConfirmDelete = () => {
    const next = credentialEditorStateAfterDeleteRequest({ draft, editingSavedCredential }, true);
    // Clear plaintext before the async credential deletion starts. A failure
    // must never restore a replacement secret to WebView state or the DOM.
    setDraft(next.draft);
    setChangedFields({});
    setEditorEpoch(current => current + 1);
    setEditingSavedCredential(next.editingSavedCredential);
    void onConfirmDelete();
  };

  const independentAsr = isIndependentAsrProfile(profile);
  const serviceIdentity = <>
    <header className="service-stage__heading">
      <h3>{independentAsr ? I18N.settings.speechRecognition : I18N.settings.voiceTranslation}</h3>
      {!independentAsr && !supportsSpeechTranslationMode(profile.provider) && (
        <SettingsHelp text={I18N.settings.textTranslationUnsupported} label={I18N.settings.helpLabel} />
      )}
    </header>
    {speechModeSettings}
    {profile.provider === "tencentCloud" && <TencentSetupHelp />}
  </>;
  const storageHelp = <CredentialStorageHelp id={noteId} profile={profile} />;


  return (
    <div className="credential-panel" aria-busy={busy}>
      <div className="service-credential-toolbar">
        {storageHelp}
        <div className="service-credential-toolbar__actions">
          {speechState === "present"
            && (
              <button
                type="button"
                className="settings-button settings-button--quiet settings-button--compact"
                disabled={disabled || confirmingDelete}
                onClick={onRequestDelete}
              >
                <Icon name="trash" />{I18N.settings.deleteCredentials}
              </button>
            )}
          {check}
        </div>
      </div>
      <section className="service-stage service-stage--integrated">
        {serviceIdentity}

        {profile.credentialState === "unavailable"
          && (feedback?.tone !== "error" || feedback.message === I18N.settings.profileActionFailed)
          && (
            <p className="credential-unavailable" role="status">
              {diagnosticCopy().storage}
            </p>
          )}

        {confirmingDelete
          && (
            <SettingsConfirmation
              message={I18N.settings.deleteCredentialsConfirm}
              disabled={disabled}
              onCancel={onCancelDelete}
              onConfirm={handleConfirmDelete}
            />
          )}

        <form className="credential-form" onSubmit={handleSubmit}>
          {editorState.error
            && <InlineFeedback tone="error">{editorState.error}<button
              type="button"
              className="settings-link"
              disabled={disabled}
              onClick={() => setEditorEpoch(current => current + 1)}
            >{I18N.settings.retryLoadingSettings}</button></InlineFeedback>}
          <div className="credential-form__fields">
            {credentialFieldsForProvider(profile.provider, profile.speechTranslationMode).map(
              (field) => {
                const copy = credentialFieldCopy(field, profile.provider);
                const fieldId = `${inputId}-${field}`;
                return (
                  <div className="settings-field" key={field}>
                    <label htmlFor={fieldId}>{copy.label}</label>
                    <div className="service-setup-control">
                      {copy.secret
                        ? <SavedCredentialInput
                          key={`${profile.id}:${editorEpoch}:${field}`}
                          id={fieldId}
                          profileId={profile.id}
                          field={field as StoredCredentialField}
                          label={copy.label}
                          hasSavedValue={savedValues?.savedFields.includes(field as StoredCredentialField) ?? false}
                          active={visible && !busy && !confirmingDelete}
                          value={draft[field]}
                          autoComplete="new-password"
                          spellCheck={false}
                          aria-describedby={noteId}
                          disabled={disabled}
                          placeholder={copy.placeholder}
                          onValueChange={(value) => {
                            setEditingSavedCredential(true);
                            setChangedFields((current) => ({ ...current, [field]: true }));
                            setDraft((current) => ({ ...current, [field]: value }));
                          }}
                        />
                        : <ConfigInput
                          copyable
                          expandable
                          id={fieldId}
                          ref={profile.provider === "deepLX" && field === "endpoint" ? endpointRef : undefined}
                          aria-invalid={field === "endpoint" && endpointInvalid ? true : undefined}
                          type="text"
                          inputMode={field === "appId" ? "numeric" : undefined}
                          value={displayedDraft[field]}
                          autoComplete="off"
                          spellCheck={false}
                          aria-describedby={field === "endpoint" && endpointInvalid
                            ? `${fieldId}-error ${noteId}` : noteId}
                          disabled={disabled}
                          placeholder={copy.placeholder}
                          onValueChange={(value) => {
                            setEditingSavedCredential(true);
                            setChangedFields((current) => ({ ...current, [field]: true }));
                            setDraft((current) => ({ ...current, [field]: value }));
                            if(field === "endpoint" && endpointInvalid) {
                              setEndpointInvalid(!deepLXEndpointIsValid(value));
                            }
                          }}
                        />}
                      {(field === "apiKey" || field === "appKey")
                        && <ServiceSetupLink serviceId={profile.provider} disabled={disabled || busy} />}
                    </div>
                    {field === "endpoint" && endpointInvalid && (
                      <span id={`${fieldId}-error`} role="alert" className="credential-unavailable">
                        {I18N.settings.deepLXEndpointInvalid}
                      </span>
                    )}
                  </div>
                );
              }
            )}
          </div>
          {saveFeedback}
          {(hasCredentialChanges || speechState !== "present")
            && <span className="credential-form__actions">
              {speechState === "present"
                && (
                  <button
                    type="button"
                    className="settings-button settings-button--quiet settings-button--compact"
                    disabled={disabled}
                    onClick={() => {
                      setDraft(emptyCredentialDraft());
                      setChangedFields({});
                      setEditingSavedCredential(false);
                      setEditorEpoch(current => current + 1);
                    }}
                  >
                    {I18N.settings.cancel}
                  </button>
                )}
              <button
                type="submit"
                className="settings-button settings-button--primary settings-button--compact"
                disabled={disabled || !credentials}
              >
                <Icon name="key" />{speechState === "present"
                  ? I18N.settings.replaceCredentials
                  : I18N.settings.saveAndUse}
              </button>
            </span>}
        </form>
      </section>
      {independentAsr
        && <TextTranslationEditor
          profile={profile}
          inputId={`${inputId}-text`}
          disabled={disabled}
          busy={busy}
          visible={visible}
          feedback={null}
          onSave={onSave}
          onRequestDelete={onRequestDelete}
          onConfirmDelete={onConfirmDelete}
          confirmingDelete={confirmingDelete}
          onCancelDelete={onCancelDelete}
          storageNoteId={noteId}
          onSaveTranslationName={onSaveTranslationName}
          textConnectionCheck={textConnectionCheck}
          sourceLanguage={sourceLanguage}
          targetLanguage={targetLanguage}
          requiresStop={requiresStop}
        />}
    </div>
  );
}

function credentialFieldCopy(field: CredentialFieldName, provider: ServiceProvider): {
  label: string;
  placeholder: string;
  secret: boolean;
} {
  switch(field) {
    case "model":
      return {
        label: I18N.settings.customSpeechModel,
        placeholder: I18N.settings.modelNamePlaceholder,
        secret: false
      };
    case "asrApiKey":
      return {
        label: I18N.settings.asrApiKey,
        placeholder: I18N.settings.apiKeyPlaceholder,
        secret: true
      };
    case "token":
      return { label: I18N.settings.deepLXToken, placeholder: "Bearer token", secret: true };
    case "apiKey":
      return {
        label: I18N.settings.apiKey,
        placeholder: I18N.settings.apiKeyPlaceholder,
        secret: true,
      };
    case "endpoint":
      return {
        label: provider === "deepLX" ? I18N.settings.deepLXEndpoint : I18N.settings.azureEndpoint,
        placeholder: provider === "deepLX"
          ? I18N.settings.serviceAddressPlaceholder
          : I18N.settings.azureEndpointPlaceholder,
        secret: false,
      };
    case "deployment":
      return {
        label: I18N.settings.deploymentName,
        placeholder: I18N.settings.deploymentNamePlaceholder,
        secret: false,
      };
    case "transcriptionDeployment":
      return {
        label: I18N.settings.transcriptionDeploymentName,
        placeholder: I18N.settings.transcriptionDeploymentNamePlaceholder,
        secret: false,
      };
    case "appId":
      return {
        label: I18N.settings.appId,
        placeholder: I18N.settings.appIdPlaceholder,
        secret: false,
      };
    case "secretId":
      return {
        label: I18N.settings.secretId,
        placeholder: I18N.settings.secretIdPlaceholder,
        secret: true,
      };
    case "secretKey":
      return {
        label: I18N.settings.secretKey,
        placeholder: I18N.settings.secretKeyPlaceholder,
        secret: true,
      };
    case "appKey":
      return {
        label: I18N.settings.appKey,
        placeholder: I18N.settings.appKeyPlaceholder,
        secret: true,
      };
  }
}
