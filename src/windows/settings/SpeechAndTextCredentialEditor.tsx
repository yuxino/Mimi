import { useEffect, useRef, useState, type ComponentProps, type FormEvent, type ReactNode } from "react";
import { AlibabaSetupLink } from "./AlibabaSetupLink";
import { ProviderIcon } from "../../components/ProviderIcon";
import { TextTranslationPresetIcon } from "../../components/TextTranslationPresetIcon";
import { ConfigInput, type ConfigInputElement } from "./ConfigInput";
import { SettingsHelp } from "./SettingsHelp";
import { CredentialStorageHelp } from "./CredentialStorageHelp";
import { Icon } from "../../components/Icon";
import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import { credentialUnavailableHelp } from "../../lib/connectionDiagnostics";
import { isChatCompletionsTranslation, textTranslationForProfile, usesAlibabaRealtime } from "../../lib/providerCapabilities";
import { CHATMOCK_DEFAULT_ENDPOINT, buildTextTranslationCredentials, deepLXEndpointIsValid, emptyCredentialDraft, openAICompatibleModelIsValid } from "../../lib/providerCredentials";
import type { ProviderCredentialsInput, ServiceProfile, SourceLanguage, TargetLanguage, TextTranslation } from "../../lib/types";
import { DestructiveConfirmation } from "./DestructiveConfirmation";
import { InlineFeedback, SettingsSelect } from "./SettingsPrimitives";
import { TextTranslationName } from "./TextTranslationName";
import { textTranslationDisplayName } from "../../lib/textTranslationName";
import type { TextTranslationNameDraft } from "../../lib/types";
import { SavedCredentialInput } from "./SavedCredentialInput";
import { useCredentialEditorState } from "./useCredentialEditorState";
import { useAppleTranslationSupport } from "./useAppleTranslationSupport";
import { AppleTranslationSettings } from "./AppleTranslationSettings";
import { OPENAI_COMPATIBLE_PRESETS, PROVIDER_COPY } from "../../lib/generated/providerCopy";
import { compatiblePresetForEndpoint } from "../../lib/textTranslationPresets";
import { TextTranslationSetupLink } from "./TextTranslationSetupLink";
import { ServiceSetupLink } from "./ServiceSetupLink";

const XAI_TRANSLATION_HELP = {
  zh: "使用 xAI 的 Grok 文字模型翻译识别结果。此处的 API Key 仅用于文字翻译，与语音识别配置分别保存。",
  "zh-TW": "使用 xAI 的 Grok 文字模型翻譯辨識結果。此處的 API Key 僅用於文字翻譯，與語音辨識設定分別儲存。",
  en: "Translate recognized text with an xAI Grok text model. This API key is saved separately for text translation, independently of recognition.",
  ja: "xAI の Grok テキストモデルで認識結果を翻訳します。この API Key はテキスト翻訳用として音声認識とは別に保存します。",
  de: "Erkannten Text mit einem Grok-Textmodell von xAI übersetzen. Dieser API-Schlüssel wird getrennt von der Spracherkennung für die Textübersetzung gespeichert.",
  ko: "xAI Grok 텍스트 모델로 인식 결과를 번역합니다. 이 API 키는 음성 인식과 별도로 텍스트 번역용으로 저장됩니다.",
  fr: "Traduisez le texte reconnu avec un modèle de texte Grok de xAI. Cette clé API est enregistrée séparément pour la traduction, indépendamment de la reconnaissance vocale.",
  th: "แปลข้อความที่รู้จำด้วยโมเดลข้อความ Grok ของ xAI โดยบันทึก API key นี้สำหรับการแปลข้อความแยกจากการรู้จำเสียง",
  ru: "Переводите распознанный текст с помощью текстовой модели xAI Grok. Этот ключ API сохраняется отдельно для перевода текста, независимо от распознавания речи.",
};

/** Shared speech/text editor. Independent text destinations own their credentials and drafts. */
export function SpeechAndTextCredentialEditor({ profile, inputId, disabled, busy, visible = true, feedback, onSave, onRequestDelete, onConfirmDelete, confirmingDelete, onCancelDelete, connectionCheck, textConnectionCheck, textOnly = false, storageNoteId, onSaveTranslationName, sourceLanguage = "auto", targetLanguage = "original", requiresStop = false, onNativeTranslationBusyChange, onNativeTranslationPrepared, translationLanguageControls }: {
  speechModeSettings?: ReactNode;
  onSaveTranslationName?: (route: TextTranslationNameDraft["route"], name: string) => Promise<unknown>;
  connectionCheck?: ReactNode | ((draft?: ProviderCredentialsInput | null, sourceLanguage?: SourceLanguage) => ReactNode);
  textConnectionCheck?: (draft?: ProviderCredentialsInput | null) => ReactNode;
  sourceLanguage?: SourceLanguage;
  targetLanguage?: TargetLanguage;
  requiresStop?: boolean;
  onNativeTranslationBusyChange?: (busy: boolean) => void;
  onNativeTranslationPrepared?: () => void;
  translationLanguageControls?: (route: TextTranslation) => ReactNode;
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
  const integratedRealtime = usesAlibabaRealtime(profile);
  const savedTranslation = textTranslationForProfile(profile);
  const [translationDraft, setTranslationDraft] = useState<TextTranslation | null>(null);
  const translation = translationDraft ?? savedTranslation;
  const nativeTranslation = translation === "apple";
  const xaiTranslation = translation === "xAI";
  const [nativeHelpOpen, setNativeHelpOpen] = useState(false);
  const apple = useAppleTranslationSupport(visible);
  const [draft, setDraft] = useState(emptyCredentialDraft);
  const [draftTranslation, setDraftTranslation] = useState(translation);
  const [editingKey, setEditingKey] = useState(false);
  const [editedFields, setEditedFields] = useState({ endpoint: false, model: false });
  const [endpointInvalid, setEndpointInvalid] = useState(false);
  const [modelInvalid, setModelInvalid] = useState(false);
  const [revealEpoch, setRevealEpoch] = useState(0);
  const [clearTranslationToken, setClearTranslationToken] = useState(false);
  const [presetDiscardedToken, setPresetDiscardedToken] = useState(false);
  const [customPreset, setCustomPreset] = useState(false);
  const recognitionState = useCredentialEditorState(profile.id, undefined, visible && !textOnly, revealEpoch);
  const translationState = useCredentialEditorState(profile.id, translation === "followService" ? undefined : translation, visible && !integratedRealtime && translation !== "followService" && !nativeTranslation, revealEpoch);
  const endpointValue = editedFields.endpoint ? draft.endpoint : translationState.state?.endpoint ?? draft.endpoint;
  const modelValue = editedFields.model ? draft.model : translationState.state?.model ?? (xaiTranslation ? "grok-4.3" : draft.model);
  const preset = translation === "openAICompatible" ? compatiblePresetForEndpoint(endpointValue) : undefined;
  const setupPreset = customPreset ? undefined : preset;
  const endpointChanged = editedFields.endpoint && draft.endpoint.trim() !== (translationState.state?.endpoint ?? "");
  const modelChanged = editedFields.model && draft.model.trim() !== (translationState.state?.model ?? "");
  const endpointDraft = translation !== savedTranslation ? endpointValue : endpointChanged ? draft.endpoint : "";
  const modelDraft = translation !== savedTranslation || xaiTranslation && profile.textCredentialState !== "present" ? modelValue : modelChanged ? draft.model : "";
  const hasTranslationKey = translationState.state?.savedFields.includes("token") ?? false;
  const metadataHasDestination = translation === "deepL" || xaiTranslation ? hasTranslationKey
    : translation === "deepLX" ? !!translationState.state?.endpoint
    : isChatCompletionsTranslation(translation) ? !!translationState.state?.endpoint && !!translationState.state?.model : false;
  const currentDestinationPresent = (profile.textCredentialState ?? profile.credentialState) === "present" && translation === savedTranslation;
  const keepsSavedText = metadataHasDestination || currentDestinationPresent;
  const endpointRef = useRef<ConfigInputElement>(null);
  const modelRef = useRef<ConfigInputElement>(null);
  const feedbackRef = useRef<HTMLDivElement>(null);
  const saved = (textOnly ? profile.textCredentialState : integratedRealtime ? profile.speechCredentialState ?? profile.credentialState : profile.credentialState) === "present" || (textOnly && (savedTranslation === "followService" || savedTranslation === "apple"));
  // Replacement drafts and saved-value previews must never carry across
  // routes, including a destination changed by another window.
  if (draftTranslation !== translation) {
    setDraftTranslation(translation);
    setDraft((current) => ({ ...current, endpoint: translation === "chatMock" && translation !== savedTranslation ? CHATMOCK_DEFAULT_ENDPOINT : "", token: "", model: translation === "xAI" ? "grok-4.3" : "" }));
    setEditedFields({ endpoint: false, model: false });
    setEndpointInvalid(false);
    setModelInvalid(false);
    setClearTranslationToken(false);
    setCustomPreset(false);
    setPresetDiscardedToken(false);
  }
  const clearToken = clearTranslationToken || presetDiscardedToken && !draft.token.trim();
  const credentials: ProviderCredentialsInput | null = integratedRealtime
    ? draft.apiKey.trim() ? { kind: "apiKey", apiKey: draft.apiKey.trim() } : null
    : buildTextTranslationCredentials(profile, { ...draft, endpoint: endpointDraft, model: modelDraft }, translation, clearToken, keepsSavedText);
  const dirty = !saved || !!draft.apiKey || translation !== savedTranslation || endpointChanged || !!draft.token || modelChanged || clearToken;
  const compatible = isChatCompletionsTranslation(translation);
  const keepsSavedDestination = keepsSavedText;
  const awaitingSavedDestination = translation !== "followService" && !nativeTranslation && (translationState.loading || !!translationState.error);
  const savedSpeech = (profile.speechCredentialState ?? profile.credentialState) === "present";
  const unavailableSpeech = (profile.speechCredentialState ?? profile.credentialState) === "unavailable";
  const speechCheckDraft: ProviderCredentialsInput | null | undefined = !draft.apiKey.trim()
    ? savedSpeech || (unavailableSpeech && !draft.apiKey) ? undefined : null
    : { kind: "alibabaTranslation", apiKey: draft.apiKey.trim(), textTranslation: "followService", endpoint: "", token: "", model: "" };
  const textChanged = translation !== savedTranslation || endpointChanged || modelChanged || !!draft.token.trim() || clearToken || (translation === "followService" && !!draft.apiKey.trim());
  let textCheckDraft: ProviderCredentialsInput | null | undefined;
  if (textChanged) {
    const endpointRequired = translation === "deepLX" || compatible;
    const invalidEndpoint = endpointRequired && (editedFields.endpoint || !keepsSavedText) && !deepLXEndpointIsValid(endpointValue);
    const invalidModel = (compatible || xaiTranslation) && (editedFields.model || !keepsSavedText) && !openAICompatibleModelIsValid(modelValue);
    const missingToken = (translation === "deepL" || xaiTranslation) && !draft.token.trim() && !keepsSavedText;
    const completeExplicitText = xaiTranslation ? !!draft.token.trim() && openAICompatibleModelIsValid(modelValue) : translation === "deepL" ? !!draft.token.trim()
      : endpointRequired && deepLXEndpointIsValid(draft.endpoint) && (!compatible || openAICompatibleModelIsValid(draft.model)) && (!!draft.token.trim() || (compatible && clearToken));
    const unavailableSavedText = !nativeTranslation && translation !== "followService" && (translationState.loading || !!translationState.error) && !completeExplicitText;
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
      model: compatible || xaiTranslation ? modelValue.trim() : "",
      token: nativeTranslation || translation === "followService" || clearToken ? "" : draft.token.trim(),
      ...(compatible && clearToken ? { clearToken: true } : {}),
    };
  }
  const destinationKeyLabel = xaiTranslation || setupPreset?.setupKind === "cloud" ? I18N.settings.apiKey : translation === "deepL" ? I18N.settings.deepLApiKey : compatible ? I18N.settings.openAICompatibleApiKey : I18N.settings.deepLXToken;
  const endpointId = `${inputId}-endpoint`;
  const modelId = `${inputId}-model`;
  const noteId = storageNoteId ?? `${inputId}-storage-note`;
  const presetCopy = PROVIDER_COPY[effectiveUiLanguage()];
  const choosePreset = (id: string) => {
    const next = OPENAI_COMPATIBLE_PRESETS.find(item => item.id === id);
    // Custom leaves the editable destination intact. Selecting a preset is
    // only a local draft change; it never saves, reveals a key or connects.
    if (!next) { setCustomPreset(true); return; }
    setCustomPreset(false);
    const changedDestination = compatiblePresetForEndpoint(endpointValue)?.id !== next.id;
    setDraft(current => ({ ...current, endpoint: changedDestination ? next.endpoint : endpointValue, model: next.model, token: changedDestination ? "" : current.token }));
    setEditedFields({ endpoint: true, model: true });
    if (changedDestination) { setClearTranslationToken(false); setPresetDiscardedToken(true); }
    setEndpointInvalid(false);
    setModelInvalid(false);
  };

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
    setCustomPreset(false);
    setPresetDiscardedToken(false);
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!credentials || disabled || awaitingSavedDestination) return;
    if ((translation === "deepLX" || compatible) && (editedFields.endpoint || !keepsSavedDestination) && !deepLXEndpointIsValid(endpointValue)) {
      setEndpointInvalid(true);
      return;
    }
    setEndpointInvalid(false);
    if ((compatible || xaiTranslation) && (editedFields.model || !keepsSavedDestination) && !openAICompatibleModelIsValid(modelValue)) {
      setModelInvalid(true);
      return;
    }
    setModelInvalid(false);
    if (!saved) setEditingKey(true);
    void onSave(credentials).then((result) => { if (result) discard(); });
  };

  const translationHelp = xaiTranslation ? XAI_TRANSLATION_HELP[effectiveUiLanguage()] : nativeTranslation ? I18N.settings.appleTranslationHelp : translation === "chatMock" ? I18N.settings.chatMockSetup : textOnly ? [I18N.settings.customSpeechTranslationHelp, ...(compatible ? [I18N.settings.openAICompatibleRequirements, I18N.settings.openAICompatibleLanguages] : [])].join("\n") : translation === "followService" ? I18N.settings.textTranslationDefault
    : compatible ? [I18N.settings.openAICompatibleChain, I18N.settings.openAICompatibleRequirements, I18N.settings.openAICompatibleLanguages].join("\n")
    : translation === "deepL" ? I18N.settings.deepLChain : I18N.settings.deepLXChain;
  const translationOptions = [
    { value: "followService", label: textOnly ? I18N.settings.customSpeechNoTranslation : I18N.settings.textTranslationFollow, icon: textOnly ? <Icon name="captions-bubble" /> : <ProviderIcon provider="alibabaCloud" size={32} /> },
    ...(apple.support?.available || savedTranslation === "apple" || nativeTranslation ? [{ value: "apple", label: I18N.settings.textTranslationApple, icon: <ProviderIcon provider="apple" size={32} /> }] : []),
    { value: "xAI", label: "xAI Grok", icon: <ProviderIcon provider="xAI" size={32} /> },
    { value: "deepL", label: "DeepL", icon: <ProviderIcon provider="deepL" size={32} /> },
    { value: "deepLX", label: I18N.settings.textTranslationCustom, icon: <ProviderIcon provider="deepLX" size={32} /> },
    { value: "openAICompatible", label: I18N.settings.textTranslationOpenAICompatible, icon: <ProviderIcon provider="openAICompatible" size={32} /> },
    { value: "chatMock", label: "ChatMock", icon: <ProviderIcon provider="chatMock" size={32} /> },
  ].map(option => option.value === "followService" ? option : { ...option, label: textTranslationDisplayName(profile, option.value as TextTranslation) });
  const nativeGuideId = `${inputId}-apple-translation-guide`;
  const translationCheck = !integratedRealtime && !(textOnly && translation === "followService") && !(nativeTranslation && (targetLanguage === "original" || sourceLanguage === targetLanguage))
    ? textConnectionCheck?.(textCheckDraft) : null;

  const stages = <>
    {!textOnly && <section className="service-stage" aria-labelledby={`${inputId}-recognition-title`}>
      <header className="service-stage__heading">
        <h3 id={`${inputId}-recognition-title`}>{integratedRealtime ? I18N.settings.voiceTranslation : I18N.settings.speechRecognition}</h3>
      </header>
      <div className="settings-field service-stage__selector">
        <span>{I18N.settings.serviceProvider}</span>
        <span className="service-stage__provider"><ProviderIcon provider="alibabaCloud" size={32} />{I18N.settings.providerAlibaba}</span>
      </div>
      {<div className="settings-field">
        <label htmlFor={`${inputId}-apiKey`}>{I18N.settings.apiKey}</label>
        <div className="alibaba-setup-control">
        <SavedCredentialInput key={`${profile.id}:${revealEpoch}`} profileId={profile.id} field={profile.provider === "deepLX" ? "asrApiKey" : "apiKey"} label={I18N.settings.apiKey} hasSavedValue={recognitionState.state?.savedFields.includes(profile.provider === "deepLX" ? "asrApiKey" : "apiKey") ?? false} active={visible && !busy && !confirmingDelete} id={`${inputId}-apiKey`} type="password" autoComplete="new-password" spellCheck={false} disabled={disabled} value={draft.apiKey} placeholder={I18N.settings.apiKeyPlaceholder} aria-describedby={noteId} onValueChange={(value) => { setEditingKey(true); setDraft((current) => ({ ...current, apiKey: value })); }} />
          {(profile.provider === "alibabaCloud" || profile.provider === "deepLX") && <AlibabaSetupLink page="apiKey" disabled={disabled || busy} />}
        </div>
      </div>}
    </section>}
    {!integratedRealtime && <section className="service-stage service-stage--translation" aria-labelledby={`${inputId}-translation-title`}>
      <header className="service-stage__heading">
        <div className="service-stage__name-help"><h3 id={`${inputId}-translation-title`}>{I18N.settings.textTranslationLabel}</h3><SettingsHelp text={translationHelp} label={I18N.settings.helpLabel} /></div>
        {nativeTranslation
          ? <button type="button" className="settings-link apple-speech-tutorial-toggle" aria-controls={nativeGuideId} aria-expanded={nativeHelpOpen} onClick={() => setNativeHelpOpen(open => !open)}>{I18N.settings.appleSpeechInstallHelp}<Icon name={nativeHelpOpen ? "chevron-up" : "chevron-down"} /></button>
          : <div className="service-stage__actions">{translationCheck}</div>}
      </header>
      {nativeTranslation && nativeHelpOpen && <div id={nativeGuideId} className="apple-speech-tutorial" role="region" aria-label={I18N.settings.appleSpeechInstallHelp}>
        <p>{I18N.settings.appleTranslationDownloadHelp}</p>
        <ol><li>{I18N.settings.appleTranslationStepLanguages}</li><li>{I18N.settings.appleTranslationStepDownload}</li><li>{I18N.settings.appleTranslationStepStart}</li></ol>
      </div>}
      <div className="settings-field service-stage__selector">
        <span>{I18N.settings.serviceProvider}</span>
        <SettingsSelect label={I18N.settings.textTranslationLabel} disabled={disabled} value={translation} options={translationOptions}
            onChange={(value) => { setTranslationDraft(value as TextTranslation); setEndpointInvalid(false); }} />
      </div>
      {translation === "followService" && translationLanguageControls?.(translation)}
      {translation !== "followService" && onSaveTranslationName && <TextTranslationName
        key={`${profile.id}:${translation}`} profile={profile} route={translation} inputId={`${inputId}-name`}
        disabled={disabled} onSave={onSaveTranslationName} />}
      {nativeTranslation && translationLanguageControls?.(translation)}
      {nativeTranslation && <AppleTranslationSettings profileId={profile.id} sourceLanguage={sourceLanguage} targetLanguage={targetLanguage}
        support={apple.support} loading={apple.loading} failed={apple.failed} visible={visible} disabled={disabled || busy} requiresStop={requiresStop}
        onRetry={apple.refresh} onBusyChange={onNativeTranslationBusyChange} onPrepared={onNativeTranslationPrepared} />}
      {nativeTranslation && <div className="apple-speech-connection-check">{translationCheck}</div>}
      {translation !== "followService" && !nativeTranslation && <div className="credential-form__fields">
        {translation === "openAICompatible" && <div className="settings-field service-stage__selector translation-preset-field">
          <span className="service-stage__name-help">{presetCopy.compatiblePresetLabel}<SettingsHelp text={setupPreset ? presetCopy[setupPreset.setupHelpKey] : presetCopy.compatiblePresetHelp} label={I18N.settings.helpLabel} /></span>
          <SettingsSelect label={presetCopy.compatiblePresetLabel} disabled={disabled || awaitingSavedDestination} value={customPreset ? "custom" : preset?.id ?? "custom"}
            options={[{ value: "custom", label: presetCopy.compatiblePresetCustom, icon: <ProviderIcon provider="openAICompatible" size={32} /> }, ...OPENAI_COMPATIBLE_PRESETS.map(item => ({ value: item.id, label: item.label, icon: <TextTranslationPresetIcon presetId={item.id} /> }))]}
            onChange={choosePreset} />
        </div>}
        {(translation === "deepLX" || compatible) && <label className="settings-field" htmlFor={endpointId}>
          <span>{compatible ? I18N.settings.openAICompatibleEndpoint : I18N.settings.deepLXEndpoint}</span>
          <ConfigInput copyable expandable key={translation} ref={endpointRef} id={endpointId} type="text" autoComplete="off" spellCheck={false} disabled={disabled} required value={endpointValue} placeholder={I18N.settings.serviceAddressPlaceholder} aria-invalid={endpointInvalid || undefined} aria-describedby={endpointInvalid ? `${endpointId}-error ${noteId}` : noteId} onValueChange={(value) => { setEditedFields((current) => ({ ...current, endpoint: true })); setDraft((current) => ({ ...current, endpoint: value })); if (endpointInvalid) setEndpointInvalid(!deepLXEndpointIsValid(value)); }} />
          {endpointInvalid && <span id={`${endpointId}-error`} role="alert" className="credential-unavailable">{I18N.settings.deepLXEndpointInvalid}</span>}
        </label>}
        {(compatible || xaiTranslation) && <label className="settings-field" htmlFor={modelId}>
          <span>{I18N.settings.openAICompatibleModel}</span>
          <div className="service-setup-control">
          <ConfigInput copyable expandable key={translation} ref={modelRef} id={modelId} type="text" autoComplete="off" spellCheck={false} disabled={disabled} required value={modelValue} placeholder={translation === "chatMock" ? I18N.settings.chatMockModelPlaceholder : I18N.settings.modelNamePlaceholder} aria-invalid={modelInvalid || undefined} aria-describedby={modelInvalid ? `${modelId}-error ${noteId}` : noteId} onValueChange={(value) => { setEditedFields((current) => ({ ...current, model: true })); setDraft((current) => ({ ...current, model: value })); if (modelInvalid) setModelInvalid(!openAICompatibleModelIsValid(value)); }} />
          {setupPreset && <TextTranslationSetupLink preset={setupPreset} page="guide" disabled={disabled || busy} />}
          </div>
          {modelInvalid && <span id={`${modelId}-error`} role="alert" className="credential-unavailable">{I18N.settings.openAICompatibleModelInvalid}</span>}
        </label>}
        <div className="settings-field">
          <span className="service-stage__field-label"><label htmlFor={`${inputId}-token`}>{destinationKeyLabel}</label>{compatible && <SettingsHelp id={`${inputId}-compatible-required`} text={keepsSavedDestination ? I18N.settings.openAICompatibleAddressKey : I18N.settings.openAICompatibleRequired} label={I18N.settings.helpLabel} />}</span>
          <div className="service-setup-control">
          <SavedCredentialInput key={`${profile.id}:${translation}:${revealEpoch}`} profileId={profile.id} field="token" textTranslation={translation} label={destinationKeyLabel} hasSavedValue={hasTranslationKey && !clearTranslationToken && !endpointChanged && !presetDiscardedToken} allowSavedReveal={translation === savedTranslation && !presetDiscardedToken} active={visible && !busy && !confirmingDelete} id={`${inputId}-token`} type="password" autoComplete="new-password" spellCheck={false} disabled={disabled || clearTranslationToken} value={draft.token} placeholder={compatible ? clearTranslationToken ? I18N.settings.noTranslationKeyPlaceholder : hasTranslationKey && !endpointChanged && !presetDiscardedToken ? I18N.settings.savedTranslationKeyPlaceholder : setupPreset?.setupKind === "cloud" ? I18N.settings.apiKeyPlaceholder : I18N.settings.optionalTranslationKeyPlaceholder : translation === "deepL" || xaiTranslation ? hasTranslationKey ? I18N.settings.savedTranslationKeyPlaceholder : I18N.settings.apiKeyPlaceholder : undefined} aria-describedby={compatible ? `${inputId}-compatible-required ${noteId}` : noteId} onValueChange={(value) => setDraft((current) => ({ ...current, token: value }))} />
          {setupPreset?.setupKind === "keyless" ? <span className="translation-setup-keyless">{presetCopy.compatibleSetupNoKey}</span>
            : setupPreset && <TextTranslationSetupLink preset={setupPreset} page="apiKey" disabled={disabled || busy} />}
          {(translation === "deepL" || xaiTranslation) && <ServiceSetupLink serviceId={xaiTranslation ? "xAIRealtime" : "deepL"} disabled={disabled || busy} />}
          </div>
        </div>
        {compatible && keepsSavedDestination && hasTranslationKey && !presetDiscardedToken && <span className="credential-form__actions">
          <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} aria-pressed={clearTranslationToken} onClick={() => { setClearTranslationToken((current) => !current); setDraft((current) => ({ ...current, token: "" })); }}>
            <Icon name="key" />{clearTranslationToken ? I18N.settings.cancelTranslationKeyRemoval : I18N.settings.removeTranslationApiKey}
          </button>
        </span>}
      </div>}
    </section>}
  </>;

  return <div className="credential-panel" aria-busy={busy}>
    {!textOnly && <div className="service-credential-toolbar">
      <CredentialStorageHelp id={noteId} profile={profile} />
      {saved && <span className="credential-panel__saved-actions">
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled || confirmingDelete} onClick={onRequestDelete}><Icon name="trash" />{I18N.settings.deleteCredentials}</button>
      </span>}

      {typeof connectionCheck === "function" ? connectionCheck(speechCheckDraft) : connectionCheck}</div>}
    {!textOnly && profile.credentialState === "unavailable" && (feedback?.tone !== "error" || feedback.message === I18N.settings.profileActionFailed) && <p role="status" className="credential-unavailable">{credentialUnavailableHelp()}</p>}
    <form className="credential-form service-stages" onSubmit={submit}>
      {stages}
      {(recognitionState.error || translationState.error) && <InlineFeedback tone="error">{recognitionState.error ?? translationState.error}<button type="button" className="settings-link" disabled={disabled} onClick={() => setRevealEpoch(current => current + 1)}>{I18N.settings.retryLoadingSettings}</button></InlineFeedback>}
      {feedback && <div ref={feedbackRef} tabIndex={-1}><InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback></div>}
      {dirty && <span className="credential-form__actions">
        {saved && <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={disabled} onClick={discard}>{I18N.settings.cancel}</button>}
        <button type="submit" className="settings-button settings-button--primary settings-button--compact" disabled={disabled || awaitingSavedDestination || !credentials || (!textOnly && editingKey && !draft.apiKey.trim())}><Icon name={nativeTranslation ? "checkmark" : "key"} />{textOnly ? I18N.settings.saveTranslationConfiguration : saved ? I18N.settings.replaceCredentials : integratedRealtime && !profile.alibabaWorkspaceId?.trim() ? I18N.settings.saveCredentials : I18N.settings.saveAndUse}</button>
      </span>}
    </form>
    {!textOnly && confirmingDelete && <DestructiveConfirmation message={I18N.settings.deleteCredentialsConfirm} disabled={disabled} onCancel={onCancelDelete} onConfirm={() => { discard(); void onConfirmDelete(); }} />}
  </div>;
}

export function TextTranslationEditor(props: Omit<ComponentProps<typeof SpeechAndTextCredentialEditor>, "textOnly">) {
  return <SpeechAndTextCredentialEditor {...props} textOnly />;
}
