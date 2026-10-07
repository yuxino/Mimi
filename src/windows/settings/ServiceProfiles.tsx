import { profileSelectionChangesSettings } from "../../lib/profileLanguagePreset";
import { testProfileConnection, type ConnectionCheckStage, type ConnectionDiagnostic, type StoredCredentialField } from "../../lib/ipc";
import { credentialErrorMessage, profileErrorMessage, diagnosticCopy } from "../../lib/connectionDiagnostics";
import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Tooltip } from "../../components/Tooltip";
import { Icon } from "../../components/Icon";
import { ProviderIcon } from "../../components/ProviderIcon";
import { I18N, providerDisplayName } from "../../lib/i18n";
import { SERVICE_PROVIDERS, credentialStateForTarget, isCustomSpeechProvider, isStandaloneAsrProvider, subtitlePreferencesChanged, textTranslationForProfile } from "../../lib/providerCapabilities";
import { DEFAULT_NETWORK_PROXY, networkProxyConfigKey } from "../../lib/networkProxy";
import {
  buildProviderCredentials,
  buildProviderProbeCredentials,
  deepLXEndpointIsValid,
  credentialEditorStateAfterDeleteRequest,
  credentialFieldsForProvider,
  emptyCredentialDraft,
  type CredentialDraft,
  type CredentialFieldName,
} from "../../lib/providerCredentials";
import { useStore } from "../../lib/store";
import type {
  NetworkProxyConfig,
  CredentialState,
  ProviderCredentialsInput,
  ServiceProfile,
  SourceLanguage,
  ProfileLanguagePreset,
  ServiceProvider,
  SessionStateEvent,
  SettingsSnapshot,
  TextTranslationNameDraft,
} from "../../lib/types";
import { InlineFeedback, SettingsSection } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

import { DestructiveConfirmation, SettingsConfirmation } from "./DestructiveConfirmation";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";
import { CustomSpeechCredentialEditor } from "./CustomSpeechCredentialEditor";

import { ConfigInput, type ConfigInputElement } from "./ConfigInput";
import { SettingsHelp } from "./SettingsHelp";
import { CredentialStorageHelp } from "./CredentialStorageHelp";
import { DraftConnectionCheck } from "./ConnectionCheck";
import { saveAndSelectProfile } from "./saveAndSelectProfile";
import { SavedCredentialInput } from "./SavedCredentialInput";
import { useCredentialEditorState } from "./useCredentialEditorState";
import { SettingsInitializationStatus } from "./SettingsInitializationStatus";
import { textTranslationDisplayName } from "../../lib/textTranslationName";
import { speechRecognitionDisplayName } from "../../lib/speechRecognitionName";
import { NetworkProxySettings } from "./NetworkProxySettings";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES } from "../../lib/types";
import { ProfileLanguagePresetSettings } from "./ProfileLanguagePresetSettings";
import { ProfileLanguageSettings } from "./ProfileLanguageSettings";
import { CustomSpeechLanguageSettings } from "./CustomSpeechLanguageSettings";
import { speechLanguageGuidance } from "../../lib/speechLanguageGuidance";
import { AutoSaveNameField } from "./AutoSaveNameField";
import { AppleSpeechSettings } from "./AppleSpeechSettings";
import { useAppleSpeechSupport } from "./useAppleSpeechSupport";
import { TencentSetupHelp } from "./TencentSetupHelp";
import { QwenMTModelSettings } from "./QwenMTModelSettings";

const CONNECTION_CHECK_TIMEOUT_MS = 30_000;

type Feedback = { tone: "success" | "error" | "info"; message: string };
type PendingAction = "create" | "select" | "delete" | "save-key" | "save-model" | "delete-key" | "test-connection" | "save-proxy" | "save-languages" | "switch-language" | "prepare-resource" | "prepare-apple-translation" | null;
type CheckStage = ConnectionCheckStage | "combined";
type CheckOutcome = { profileId: string; input: symbol; result: ConnectionDiagnostic | null; error: string | null };
type PendingConfirmation =
  | { kind: "profile"; profileId: string; name: string }
  | { kind: "credential"; profileId: string }
  | null;

export function ServiceProfiles({
  settings,
  sessionIsActive,
  sessionIsPaused = false,
  sessionStatusKind = "idle",
  visible = true,
  overview,
  appleResourcesRequest = 0,
  profileEditorRequest = 0,
}: {
  settings: SettingsSnapshot;
  sessionIsActive: boolean;
  sessionIsPaused?: boolean;
  sessionStatusKind?: SessionStateEvent["status"]["kind"];
  visible?: boolean;
  overview?: ReactNode;
  appleResourcesRequest?: number;
  profileEditorRequest?: number;
}) {
  const createProfile = useStore((state) => state.createProfile);
  const updateProfile = useStore((state) => state.updateProfile);
  const selectProfile = useStore((state) => state.selectProfile);
  const deleteProfile = useStore((state) => state.deleteProfile);
  const saveProfileCredentials = useStore((state) => state.saveProfileCredentials);
  const deleteProfileAPIKey = useStore((state) => state.deleteProfileAPIKey);
  const initializationStatus = useStore((state) => state.initializationStatus) ?? "ready";
  const initializationError = useStore((state) => state.initializationError) ?? null;
  const initialize = useStore((state) => state.init);
  const apple = useAppleSpeechSupport(visible);
  const canUseProfile = (profile: ServiceProfile) => profile.provider === "appleSpeech"
    ? apple.support?.available === true
    : credentialStateForTarget(profile, profile.languagePreset?.targetLanguage ?? settings.targetLanguage) === "present";

  const activeProfile =
    settings.profiles.find((profile) => profile.id === settings.activeProfileId) ??
    settings.profiles[0];
  const [selectedProfileId, setSelectedProfileId] = useState(
    activeProfile?.id ?? settings.activeProfileId,
  );
  const [showsEditor, setShowsEditor] = useState(false);
  const [handledProfileEditorRequest, setHandledProfileEditorRequest] = useState(0);
  const [deferredProfileEditorRequest, setDeferredProfileEditorRequest] = useState(0);
  const [openedProfileEditorRequest, setOpenedProfileEditorRequest] = useState(0);
  const focusedProfileEditorRequest = useRef(0);
  const profileDetailHeading = useRef<HTMLHeadingElement>(null);
  const [handledAppleResourcesRequest, setHandledAppleResourcesRequest] = useState(0);
  const focusedAppleResourcesRequest = useRef(0);
  const [appleResourcesProfileId, setAppleResourcesProfileId] = useState<string | null>(null);
  const [showsProviderPicker, setShowsProviderPicker] = useState(false);
  const [pendingAction, setPendingAction] = useState<PendingAction>(null);
  const [profileOperationFailed, setProfileOperationFailed] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const { beginToast } = useSettingsToast();
  const [diagnostics, setDiagnostics] = useState<Partial<Record<CheckStage, CheckOutcome>>>({});
  const [pendingCheckStage, setPendingCheckStage] = useState<CheckStage | null>(null);
  const creationInFlight = useRef(false);
  const mutationInFlight = useRef(false);
  const proxyKeys = useMemo(() => Object.fromEntries(settings.profiles.map(profile => {
    const speech = networkProxyConfigKey(profile.speechNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY);
    const text = networkProxyConfigKey(profile.textNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY);
    return [profile.id, { speech, text, combined: JSON.stringify([speech, text]) }];
  })), [settings.profiles, settings.networkProxy]);
  const proxyKey = JSON.stringify(proxyKeys);
  const [renderedProxies, setRenderedProxies] = useState({ key: proxyKey, keys: proxyKeys });
  const latestProxyKeys = useRef(proxyKeys);
  useEffect(() => { latestProxyKeys.current = proxyKeys; }, [proxyKeys]);
  const [renderedSession, setRenderedSession] = useState({
    kind: sessionStatusKind,
    profileId: settings.activeProfileId,
  });
  const [pendingConfirmation, setPendingConfirmation] = useState<PendingConfirmation>(null);
  const mounted = useRef(false);
  const checkRequest = useRef(0);
  const checkInFlight = useRef(false);
  const checkTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const profileCheckEpochs = useRef(new Map<string, number>());
  const [renderedProfileId, setRenderedProfileId] = useState(activeProfile?.id);

  const selectedProfile = useMemo(
    () => settings.profiles.find((profile) => profile.id === selectedProfileId) ?? activeProfile,
    [activeProfile, selectedProfileId, settings.profiles],
  );
  const selectedProfileReadOnly = selectedProfile?.credentialStorage === "localDevFile";

  const invalidateProfileCheck = useCallback((profileId: string, stage?: ConnectionCheckStage) => {
    const epochs = profileCheckEpochs.current;
    epochs.set(profileId, (epochs.get(profileId) ?? 0) + 1);
    setDiagnostics((current) => Object.fromEntries(Object.entries(current).filter(([key, outcome]) => outcome?.profileId !== profileId || (stage && key !== stage && key !== "combined"))));
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      checkRequest.current += 1;
      checkInFlight.current = false;
      if (checkTimer.current !== null) clearTimeout(checkTimer.current);
    };
  }, []);

  if (renderedProxies.key !== proxyKey) {
    const previous = renderedProxies.keys;
    setRenderedProxies({ key: proxyKey, keys: proxyKeys });
    setDiagnostics(current => Object.fromEntries(Object.entries(current).filter(([stage, outcome]) => outcome && previous[outcome.profileId]?.[stage as CheckStage] === proxyKeys[outcome.profileId]?.[stage as CheckStage])));
  }

  useEffect(() => {
    if (sessionStatusKind !== "error") return;
    const epochs = profileCheckEpochs.current;
    epochs.set(settings.activeProfileId, (epochs.get(settings.activeProfileId) ?? 0) + 1);
  }, [sessionStatusKind, settings.activeProfileId]);

  if (renderedSession.kind !== sessionStatusKind || renderedSession.profileId !== settings.activeProfileId) {
    setRenderedSession({ kind: sessionStatusKind, profileId: settings.activeProfileId });
    if (sessionStatusKind === "error") {
      setDiagnostics((current) => Object.fromEntries(Object.entries(current).filter(([, outcome]) => outcome?.profileId !== settings.activeProfileId)));
    }
  }

  if (
    selectedProfile &&
    selectedProfile.id !== renderedProfileId
  ) {
    setRenderedProfileId(selectedProfile.id);
    setFeedback(null);
    setPendingConfirmation(null);
  }

  if (visible && profileEditorRequest > 0 && profileEditorRequest !== handledProfileEditorRequest
    && pendingAction !== null && deferredProfileEditorRequest !== profileEditorRequest) {
    setDeferredProfileEditorRequest(profileEditorRequest);
  }

  // Open the currently active configuration, even if another editor or the
  // provider picker was left open. Defer navigation until a pending save ends.
  if (visible && initializationStatus === "ready" && pendingAction === null
    && profileEditorRequest !== handledProfileEditorRequest) {
    setHandledProfileEditorRequest(profileEditorRequest);
    // A failed deferred save must keep its draft and retry visible. A new
    // explicit navigation request can still leave that editor afterwards.
    if (profileEditorRequest > 0 && activeProfile
      && !(deferredProfileEditorRequest === profileEditorRequest && (profileOperationFailed || feedback?.tone === "error"))) {
      setOpenedProfileEditorRequest(profileEditorRequest);
      setSelectedProfileId(activeProfile.id);
      setShowsProviderPicker(false);
      setShowsEditor(true);
      setPendingConfirmation(null);
      setFeedback(null);
    }
  }

  useEffect(() => {
    if (!visible || !showsEditor || !activeProfile || selectedProfile?.id !== activeProfile.id
      || pendingAction !== null || profileEditorRequest === 0
      || openedProfileEditorRequest !== profileEditorRequest
      || focusedProfileEditorRequest.current === profileEditorRequest) return;
    const frame = window.requestAnimationFrame(() => {
      if (profileDetailHeading.current) {
        profileDetailHeading.current.focus({ preventScroll: true });
        focusedProfileEditorRequest.current = profileEditorRequest;
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [activeProfile, openedProfileEditorRequest, pendingAction, profileEditorRequest, selectedProfile?.id, showsEditor, visible]);

  // A native resource action targets the current Apple profile, even when this
  // window last showed another editor. Do not unmount a pending form operation.
  if (visible && initializationStatus === "ready" && pendingAction === null
    && appleResourcesRequest !== handledAppleResourcesRequest) {
    setHandledAppleResourcesRequest(appleResourcesRequest);
    setAppleResourcesProfileId(appleResourcesRequest > 0 && activeProfile?.provider === "appleSpeech" ? activeProfile.id : null);
    if (appleResourcesRequest > 0 && activeProfile?.provider === "appleSpeech") {
      setSelectedProfileId(activeProfile.id);
      setShowsProviderPicker(false);
      setShowsEditor(true);
      setPendingConfirmation(null);
    }
  }

  useEffect(() => {
    if (!visible || !showsEditor || !appleResourcesProfileId || selectedProfile?.id !== appleResourcesProfileId
      || selectedProfile.id !== settings.activeProfileId
      || selectedProfile?.provider !== "appleSpeech" || pendingAction !== null
      || appleResourcesRequest === 0 || handledAppleResourcesRequest !== appleResourcesRequest
      || focusedAppleResourcesRequest.current === appleResourcesRequest) return;
    const frame = window.requestAnimationFrame(() => {
      if (focusAppleSpeechResources()) focusedAppleResourcesRequest.current = appleResourcesRequest;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [appleResourcesRequest, appleResourcesProfileId, handledAppleResourcesRequest, pendingAction, selectedProfile?.id,
    selectedProfile?.provider, settings.activeProfileId, showsEditor, visible]);

  const hasIndependentTextProxy = !!selectedProfile && (isStandaloneAsrProvider(selectedProfile.provider) || ["alibabaCloud", "deepLX"].includes(selectedProfile.provider));
  const hasRemoteTextProxy = hasIndependentTextProxy && selectedProfile && textTranslationForProfile(selectedProfile) !== "apple";

  const SelectedCredentialEditor = selectedProfile?.provider === "appleSpeech" ? AppleSpeechSettings : selectedProfile && isCustomSpeechProvider(selectedProfile.provider) ? CustomSpeechCredentialEditor : selectedProfile && ["alibabaCloud", "deepLX"].includes(selectedProfile.provider) ? AlibabaCredentialEditor : CredentialEditor;

  const requiresStop = sessionIsActive || sessionIsPaused || sessionStatusKind === "connecting" || sessionStatusKind === "stopping";
  const mutationsDisabled = requiresStop || pendingAction !== null;
  const selectionDisabled = sessionStatusKind === "connecting" || sessionStatusKind === "stopping" || pendingAction !== null;
  const atProfileLimit = settings.profiles.filter(profile => profile.credentialStorage !== "localDevFile").length >= 20;

  const trackProfileOperation = async (operation: () => Promise<SettingsSnapshot>) => {
    setProfileOperationFailed(false);
    try {
      return await operation();
    } catch (error) {
      if (mounted.current) setProfileOperationFailed(true);
      throw error;
    }
  };

  const perform = async (
    action: Exclude<PendingAction, null>,
    operation: () => Promise<SettingsSnapshot>,
    successFeedback: string | ((snapshot: SettingsSnapshot) => Feedback),
  ): Promise<SettingsSnapshot | null> => {
    if (mutationInFlight.current) return null;
    mutationInFlight.current = true;
    if (action === "delete-key") invalidateProfileCheck(selectedProfileId);
    setPendingAction(action);
    setFeedback(null);
    const notify = beginToast();
    try {
      const snapshot = await trackProfileOperation(operation);
      const result: Feedback = typeof successFeedback === "string"
          ? { tone: "success", message: successFeedback }
          : successFeedback(snapshot);
      if (result.tone !== "error") notify(result.message);
      else setFeedback(result);
      return snapshot;
    } catch (error) {
      // Keep unsaved edits actionable in their form; list operations have no field to correct.
      if (action === "save-key") setFeedback({ tone: "error", message: profileErrorMessage(error) });
      else notify(profileErrorMessage(error), true);
      return null;
    } finally {
      mutationInFlight.current = false;
      setPendingAction(null);
    }
  };

  const handleCreate = async (provider: ServiceProvider) => {
    if (creationInFlight.current || mutationsDisabled || atProfileLimit) return;
    if (provider === "appleSpeech" && !apple.support?.available) return;
    creationInFlight.current = true;
    try {
    const previousIds = new Set(settings.profiles.map((profile) => profile.id));
    const name = providerDisplayName(provider);
    const snapshot = await perform(
      "create",
      () => createProfile(provider, name),
      I18N.settings.profileCreated,
    );
    if (!snapshot) return;
    const created = snapshot.profiles.find((profile) => !previousIds.has(profile.id));
    if (created) {
      setSelectedProfileId(created.id);
      setShowsEditor(true);
    }
    setShowsProviderPicker(false);
    } finally { creationInFlight.current = false; }
  };

  const handleRename = async (profileId: string, name: string) => {
    return updateProfile(profileId, name);
  };

  const handleSaveTranslationName = async (profile: ServiceProfile, route: TextTranslationNameDraft["route"], name: string) => {
    return updateProfile(profile.id, undefined, { textTranslationName: { route, name } });
  };

  const handleSaveProxy = async (profile: ServiceProfile, stage: "speech" | "text", config: NetworkProxyConfig) => {
    if (mutationInFlight.current || mutationsDisabled) throw new Error("network_proxy_change_requires_stop");
    mutationInFlight.current = true;
    setPendingAction("save-proxy");
    try {
      await trackProfileOperation(() => updateProfile(profile.id, undefined, stage === "speech" ? { speechNetworkProxy: config } : { textNetworkProxy: config }));
      invalidateProfileCheck(profile.id, stage);
    } finally {
      mutationInFlight.current = false;
      setPendingAction(null);
    }
  };

  const handleSaveLanguagePreset = async (preset: ProfileLanguagePreset | null) => {
    if (!selectedProfile || mutationInFlight.current || mutationsDisabled || selectedProfileReadOnly) throw new Error("profile-change-requires-stop");
    mutationInFlight.current = true;
    setPendingAction("save-languages");
    try {
      return await trackProfileOperation(() => updateProfile(selectedProfile.id, undefined, { languagePreset: preset }));
    } finally {
      mutationInFlight.current = false;
      if (mounted.current) setPendingAction(null);
    }
  };

  const handleSaveSpeechLanguages = async (profile: ServiceProfile, languages: SourceLanguage[] | null) => {
    if (mutationInFlight.current || mutationsDisabled || selectedProfileReadOnly) throw new Error("profile-change-requires-stop");
    mutationInFlight.current = true;
    setPendingAction("save-languages");
    const before = settings;
    const notify = beginToast();
    try {
      // The command response acknowledges this exact save even if its settings
      // event arrives first. Never infer normalization from an unrelated event.
      const after = await trackProfileOperation(() => updateProfile(profile.id, undefined, { customSpeechSourceLanguages: languages }));
      if (mounted.current) {
        invalidateProfileCheck(profile.id);
        notify(profile.id === before.activeProfileId && after.activeProfileId === before.activeProfileId && after.sourceLanguage !== before.sourceLanguage
          ? I18N.settings.recognitionLanguageAdjusted(speechLanguageGuidance(before).optionLabel(before.sourceLanguage), speechLanguageGuidance(after).optionLabel(after.sourceLanguage))
          : I18N.settings.customSpeechLanguagesSaved);
      }
      return after;
    } finally {
      mutationInFlight.current = false;
      if (mounted.current) setPendingAction(null);
    }
  };

  const handleSelect = async (profileId: string) => {
    if (selectionDisabled || mutationInFlight.current || !settings.profiles.some(profile => profile.id === profileId && profileSelectionChangesSettings(profile, settings))) return;
    setPendingConfirmation(null);
    setSelectedProfileId(profileId);
    await perform(
      "select",
      () => selectProfile(profileId),
      (snapshot) =>
        settings.profiles.find(profile => profile.id === profileId)?.languagePreset
          ? { tone: "success", message: I18N.settings.profileLanguagesApplied }
          : subtitlePreferencesChanged(settings, snapshot)
          ? {
              tone: "info",
              message: I18N.settings.profileSelectedWithAdjustments,
            }
          : {
              tone: "success",
              message: I18N.settings.profileSelected,
            },
    );
  };

  const requestProfileDelete = () => {
    if (!selectedProfile || settings.profiles.length <= 1) return;
    setPendingConfirmation({
      kind: "profile",
      profileId: selectedProfile.id,
      name: selectedProfile.name,
    });
  };

  const confirmProfileDelete = async () => {
    if (
      !selectedProfile ||
      settings.profiles.length <= 1 ||
      pendingConfirmation?.kind !== "profile" ||
      pendingConfirmation.profileId !== selectedProfile.id
    )
      return;
    setPendingConfirmation(null);
    const snapshot = await perform(
      "delete",
      () => deleteProfile(selectedProfile.id),
      I18N.settings.profileDeleted,
    );
    if (snapshot) {
      setSelectedProfileId(snapshot.activeProfileId);
      setShowsEditor(false);
    }
  };

  const handleSaveCredential = async (profileId: string, credentials: ProviderCredentialsInput) => {
    setPendingConfirmation(null);
    const stage = credentials.kind === "customSpeech" ? "speech" : credentials.kind === "alibabaTranslation" && (isStandaloneAsrProvider(selectedProfile!.provider) || !credentials.apiKey.trim()) ? "text" : undefined;
    invalidateProfileCheck(profileId, stage);
    return perform(
      "save-key",
      () => saveAndSelectProfile(profileId, credentials, saveProfileCredentials, selectProfile),
      (snapshot) => ({
        tone: subtitlePreferencesChanged(settings, snapshot) ? "info" : "success",
        message: subtitlePreferencesChanged(settings, snapshot)
          ? I18N.settings.profileSelectedWithAdjustments
          : credentials.kind === "alibabaTranslation" && credentials.textTranslation === "apple" && !credentials.apiKey.trim()
          ? I18N.settings.translationConfigurationSaved
          : I18N.settings.credentialsSaved,
      }),
    );
  };

  const requestCredentialDelete = (profileId: string) => {
    setPendingConfirmation({ kind: "credential", profileId });
  };

  const confirmCredentialDelete = async (profileId: string) => {
    if (pendingConfirmation?.kind !== "credential" || pendingConfirmation.profileId !== profileId)
      return;
    setPendingConfirmation(null);
    await perform(
      "delete-key",
      () => deleteProfileAPIKey(profileId),
      I18N.settings.credentialsDeleted,
    );
  };

  const openEditor = (profileId: string) => {
    setSelectedProfileId(profileId);
    setPendingConfirmation(null);
    setFeedback(null);
    setShowsEditor(true);
  };

  const handleConnectionCheck = (input: symbol, stage?: ConnectionCheckStage, credentials?: ProviderCredentialsInput, sourceLanguage?: SourceLanguage) => {
    if (!selectedProfile || pendingAction !== null || checkInFlight.current) return;
    checkInFlight.current = true;
    const profileId = selectedProfile.id;
    const key = stage ?? "combined";
    const publish = (result: ConnectionDiagnostic | null, error: string | null) => setDiagnostics(current => ({ ...current, [key]: { profileId, input, result, error } }));
    const request = ++checkRequest.current;
    const epoch = profileCheckEpochs.current.get(profileId) ?? 0;
    const checkedProxyKey = proxyKeys[profileId]?.[key];
    const canPublish = () => mounted.current &&
      request === checkRequest.current &&
      checkedProxyKey === latestProxyKeys.current[profileId]?.[key] &&
      epoch === (profileCheckEpochs.current.get(profileId) ?? 0);
    setPendingAction("test-connection");
    setPendingCheckStage(key);
    setDiagnostics(current => ({ ...current, [key]: undefined }));
    checkTimer.current = setTimeout(() => {
      if (!mounted.current || request !== checkRequest.current) return;
      const publishTimeout = canPublish();
      checkRequest.current += 1;
      checkInFlight.current = false;
      checkTimer.current = null;
      if (publishTimeout) publish(null, I18N.settings.profileCheckTimedOut);
      setPendingCheckStage(null);
      setPendingAction(null);
    }, CONNECTION_CHECK_TIMEOUT_MS);
    void (sourceLanguage ? testProfileConnection(profileId, stage, credentials, sourceLanguage) : credentials ? testProfileConnection(profileId, stage, credentials) : stage ? testProfileConnection(profileId, stage) : testProfileConnection(profileId))
      .then((result) => {
        if (canPublish()) publish(result, null);
      })
      .catch((error: unknown) => {
        if (canPublish()) publish(null, credentialErrorMessage(error) ?? `${diagnosticCopy().unavailable}: ${diagnosticCopy().checkFailed}`);
      })
      .finally(() => {
        if (mounted.current && request === checkRequest.current) {
          if (checkTimer.current !== null) clearTimeout(checkTimer.current);
          checkTimer.current = null;
          checkInFlight.current = false;
          setPendingCheckStage(null);
          setPendingAction(null);
        }
      });
  };

  const renderConnectionCheck = (profile: ServiceProfile, stage?: ConnectionCheckStage, draft?: ProviderCredentialsInput | null, sourceLanguage?: SourceLanguage) => {
    const key = stage ?? "combined";
    const outcome = diagnostics[key];
    const context = JSON.stringify([profile.provider, textTranslationForProfile(profile), sourceLanguage ?? settings.sourceLanguage,
      stage === "speech" ? null : settings.targetLanguage]);
    return <DraftConnectionCheck key={`${profile.id}:${key}`} draft={draft} context={context}
      outcome={outcome?.profileId === profile.id ? outcome : undefined}
      pending={pendingCheckStage === key} disabled={mutationsDisabled || (profile.provider === "appleSpeech" && stage === "speech" && apple.loading)}
      onCheck={input => { if (draft !== null) handleConnectionCheck(input, stage, draft, sourceLanguage); }}
      label={stage === "text" ? I18N.settings.checkTextTranslation : stage === "speech" ? I18N.settings.checkSpeechRecognition : undefined} />;
  };

  if (initializationStatus !== "ready") {
    return <SettingsInitializationStatus status={initializationStatus} error={initializationError} onRetry={() => { void initialize(); }} />;
  }

  return (
    <>
    {!showsEditor && !showsProviderPicker && overview}
    <SettingsSection id="service-profiles" title={I18N.settings.serviceProfilesTitle} hideHeading>
      {showsProviderPicker ? (
        <ProviderPicker
          apple={apple}
          disabled={mutationsDisabled}
          cancelDisabled={pendingAction !== null}
          requiresStop={requiresStop}
          feedback={feedback}
          onChoose={(provider) => void handleCreate(provider)}
          onCancel={() => setShowsProviderPicker(false)}
        />
      ) : showsEditor && selectedProfile ? (
        <div className="service-detail">
          <div className="service-detail__header">
            <button
              type="button"
              className="settings-button settings-button--quiet service-back"
              disabled={pendingAction !== null}
              onClick={() => {
                setShowsEditor(false);
                setPendingConfirmation(null);
                setFeedback(null);
              }}
            >
              <Icon name="chevron-left" />
              {I18N.settings.backToServices}
            </button>
            <div className="service-detail__identity">
              <ProviderIcon provider={selectedProfile.provider === "deepLX" ? "alibabaCloud" : selectedProfile.provider} />
              <div className="service-detail__copy">
                <div className="service-detail__title">
                  <div className="service-detail__name-help"><h2 ref={profileDetailHeading} tabIndex={-1} key={selectedProfile.name}>{profileTitle(selectedProfile)}</h2><SettingsHelp text={profileDescription(selectedProfile)} label={I18N.settings.helpLabel} /></div>
                  <div className="service-detail__status">
                    <CredentialBadge state={credentialStateForTarget(selectedProfile, settings.targetLanguage)} nativeSpeech={selectedProfile.provider === "appleSpeech"} />
                    {selectedProfile.id === settings.activeProfileId && (
                      <span className="profile-active-badge">
                        <Icon name="checkmark" />
                        {I18N.settings.activeProfile}
                      </span>
                    )}
                  </div>
                </div>
              </div>
            </div>
          </div>
          <div className="service-detail__configuration">
          <div className="profile-form service-detail__name">
            <AutoSaveNameField key={selectedProfile.id} id={`profile-name-${selectedProfile.id}`}
              label={I18N.settings.profileName} value={selectedProfile.name} allowEmpty={false}
              placeholder={I18N.settings.profileNamePlaceholder} disabled={mutationsDisabled} readOnly={selectedProfileReadOnly}
              onSave={name => handleRename(selectedProfile.id, name)} />
          </div>
          <div className="service-detail__connection">
            <SelectedCredentialEditor
              support={apple.support} settings={settings} requiresStop={requiresStop} loading={apple.loading} failed={apple.failed} sourceLanguage={settings.sourceLanguage} targetLanguage={settings.targetLanguage}
              resourceRefreshDisabled={pendingAction !== null}
              languageChangesDisabled={selectionDisabled}
              onRetry={apple.refresh} onPrepared={apple.update} onBusyChange={busy => {
                if (busy) invalidateProfileCheck(selectedProfile.id, "speech");
                setPendingAction(busy ? "prepare-resource" : null);
              }}
              onNativeTranslationBusyChange={busy => setPendingAction(busy ? "prepare-apple-translation" : null)}
              onNativeTranslationPrepared={() => invalidateProfileCheck(selectedProfile.id, "text")}
              translationLanguageControls={route => route === "followService" && selectedProfile.provider === "alibabaCloud"
                ? <QwenMTModelSettings profile={selectedProfile} disabled={mutationsDisabled}
                  onSave={async model => {
                    if (mutationInFlight.current || mutationsDisabled) throw new Error("profile-change-requires-stop");
                    mutationInFlight.current = true;
                    setPendingAction("save-model");
                    try {
                      await trackProfileOperation(() => updateProfile(selectedProfile.id, undefined, { qwenMtModel: model }));
                      invalidateProfileCheck(selectedProfile.id, "text");
                    } finally {
                      mutationInFlight.current = false;
                      setPendingAction(null);
                    }
                  }} />
                : route === "apple" && textTranslationForProfile(selectedProfile) === "apple" && selectedProfile.id === settings.activeProfileId
                ? <ProfileLanguageSettings settings={settings} disabled={selectionDisabled} onBusyChange={busy => setPendingAction(busy ? "switch-language" : null)} hideSourceLanguage={selectedProfile.provider === "appleSpeech"} embedded /> : null}
              onSelectProfile={async (profileId: string, sourceLanguage: SourceLanguage) => {
                if (selectionDisabled || mutationInFlight.current) throw new Error("profile_switch_busy");
                mutationInFlight.current = true;
                try {
                  await selectProfile(profileId, sourceLanguage);
                  invalidateProfileCheck(profileId);
                } finally { mutationInFlight.current = false; }
              }}
              connectionCheck={(draft: ProviderCredentialsInput | null | undefined, sourceLanguage?: SourceLanguage) => renderConnectionCheck(selectedProfile, isStandaloneAsrProvider(selectedProfile.provider) || ["alibabaCloud", "deepLX"].includes(selectedProfile.provider) ? "speech" : undefined, draft, sourceLanguage)}
              textConnectionCheck={(draft) => renderConnectionCheck(selectedProfile, "text", draft)}
              readOnly={selectedProfileReadOnly}
              key={selectedProfile.id}
              profile={selectedProfile}
              inputId={`profile-api-key-${selectedProfile.id}`}
              disabled={mutationsDisabled}
              busy={pendingAction === "save-key" || pendingAction === "delete-key"}
              visible={visible && pendingConfirmation === null}
              feedback={feedback}
              onSaveTranslationName={(route, name) => handleSaveTranslationName(selectedProfile, route, name)}
              onSaveRecognitionName={(name: string) => updateProfile(selectedProfile.id, undefined, { speechRecognitionName: name })}
              onSave={(replacement) => handleSaveCredential(selectedProfile.id, replacement)}
              onRequestDelete={() => requestCredentialDelete(selectedProfile.id)}
              onConfirmDelete={() => confirmCredentialDelete(selectedProfile.id)}
              confirmingDelete={
                pendingConfirmation?.kind === "credential" &&
                pendingConfirmation.profileId === selectedProfile.id
              }
              onCancelDelete={() => setPendingConfirmation(null)}
            />
          </div>
          {isCustomSpeechProvider(selectedProfile.provider) && <CustomSpeechLanguageSettings key={`${selectedProfile.id}-languages`}
            profile={selectedProfile} disabled={mutationsDisabled || selectedProfileReadOnly}
            onSave={languages => handleSaveSpeechLanguages(selectedProfile, languages)} />}
          {(selectedProfile.provider !== "appleSpeech" || hasRemoteTextProxy) && <section className="service-proxies" aria-label={I18N.settings.networkProxyTitle}>
            <h3>{I18N.settings.networkProxyTitle}</h3>
            {selectedProfile.provider !== "appleSpeech" && <NetworkProxySettings key={`${selectedProfile.id}-speech-proxy`} embedded
              label={hasIndependentTextProxy ? I18N.settings.speechRecognition : I18N.settings.voiceTranslation}
              scope={hasIndependentTextProxy ? I18N.settings.networkProxySpeechScope : I18N.settings.networkProxyIntegratedScope}
              value={selectedProfile.speechNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY} disabled={mutationsDisabled}
              onSave={config => handleSaveProxy(selectedProfile, "speech", config)} />}
            {hasRemoteTextProxy && <NetworkProxySettings key={`${selectedProfile.id}-text-proxy`} embedded
              label={I18N.settings.textTranslationLabel} scope={I18N.settings.networkProxyTextScope}
              value={selectedProfile.textNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY} disabled={mutationsDisabled}
              onSave={config => handleSaveProxy(selectedProfile, "text", config)} />}
          </section>}
          {selectedProfile.id === settings.activeProfileId && textTranslationForProfile(selectedProfile) !== "apple"
            ? <ProfileLanguageSettings key={selectedProfile.id} settings={settings} disabled={selectionDisabled} onBusyChange={busy => setPendingAction(busy ? "switch-language" : null)} onOpenAppleResources={focusAppleSpeechResources} hideSourceLanguage={selectedProfile.provider === "appleSpeech"} />
            : null}
          <ProfileLanguagePresetSettings key={`${selectedProfile.id}-language-preset`} profile={selectedProfile} settings={settings}
            disabled={mutationsDisabled || selectedProfileReadOnly} onSave={handleSaveLanguagePreset} />
          <div className="service-detail__actions">
            {canUseProfile(selectedProfile) &&
              profileSelectionChangesSettings(selectedProfile, settings) && (
                <span className="service-detail__use">
                <button
                  type="button"
                  className="settings-button settings-button--quiet settings-button--compact"
                  disabled={selectionDisabled}
                  onClick={() => void handleSelect(selectedProfile.id)}
                >
                  <Icon name="checkmark" />
                  {I18N.settings.useProfile}
                </button>
                <SettingsHelp text={I18N.settings.profileSwitchHelp} label={I18N.settings.helpLabel} />
                </span>
              )}
            {!selectedProfileReadOnly && <button
              type="button"
              className="settings-button settings-button--danger settings-button--compact"
              disabled={mutationsDisabled || settings.profiles.length <= 1}
              onClick={requestProfileDelete}
            >
              <Icon name="trash" />
              {I18N.settings.deleteProfile}
            </button>}
          </div>
          {pendingConfirmation?.kind === "profile" &&
            pendingConfirmation.profileId === selectedProfile.id && (
              <DestructiveConfirmation
                message={I18N.settings.deleteProfileConfirm(pendingConfirmation.name)}
                disabled={mutationsDisabled}
                onCancel={() => setPendingConfirmation(null)}
                onConfirm={() => void confirmProfileDelete()}
              />
            )}
          </div>
        </div>
      ) : (
        <div className="services-home">
          <div className="services-toolbar">
            <span className="services-toolbar__count">{I18N.settings.profileCount(settings.profiles.length)}<SettingsHelp label={I18N.settings.helpLabel} text={`${settings.profiles.some(profile => profile.credentialStorage === "localDevFile") ? diagnosticCopy().localDevReadOnly : I18N.settings.servicesHint}\n${I18N.settings.profileSwitchHelp}`} /></span>
            <button
              type="button"
              className="settings-button settings-button--compact settings-button--quiet"
              disabled={mutationsDisabled || atProfileLimit}
              onClick={() => { setFeedback(null); setShowsProviderPicker(true); }}
            >
              <Icon name="plus" />
              {I18N.settings.addProfile}
            </button>
          </div>
          <div className="service-rows" aria-label={I18N.settings.serviceProfilesTitle}>
            {settings.profiles.map((profile) => (
              <div
                className="service-row"
                key={profile.id}
                data-active={profile.id === settings.activeProfileId}
              >
                <button
                  type="button"
                  className="service-row__main"
                  disabled={selectionDisabled}
                  onClick={() => {
                    if (
                      canUseProfile(profile) &&
                      profileSelectionChangesSettings(profile, settings)
                    )
                      void handleSelect(profile.id);
                    else openEditor(profile.id);
                  }}
                  aria-label={`${profile.name}${isCustomSpeechProvider(profile.provider) && profile.speechRecognitionName?.trim() ? `, ${I18N.settings.speechRecognition}: ${speechRecognitionDisplayName(profile)}` : ""}, ${credentialStateText(credentialStateForTarget(profile, profile.languagePreset?.targetLanguage ?? settings.targetLanguage), profile.provider === "appleSpeech")}${textTranslationForProfile(profile) !== "followService" ? `, ${I18N.settings.textTranslationLabel}: ${textTranslationDisplayName(profile)}` : ""}: ${canUseProfile(profile) && profileSelectionChangesSettings(profile, settings) ? I18N.settings.useProfile : I18N.settings.editProfile}`}
                >
                  <ProviderIcon provider={profile.provider === "deepLX" ? "alibabaCloud" : profile.provider} />
                  <span className="service-row__copy">
                    <strong key={profile.name}>{profileTitle(profile)}</strong>
                    {profile.languagePreset && <span className="service-row__provider">{SOURCE_LANGUAGE_DISPLAY_NAMES[profile.languagePreset.sourceLanguage]} → {TARGET_LANGUAGE_DISPLAY_NAMES[profile.languagePreset.targetLanguage]}</span>}
                    {profileSecondaryLabel(profile) && <span className="service-row__provider">{profileSecondaryLabel(profile)}</span>}
                    {textTranslationForProfile(profile) !== "followService" && <span className="service-row__translation"><ProviderIcon provider={textTranslationForProfile(profile) as "deepL" | "deepLX" | "openAICompatible" | "chatMock" | "apple"} size={32} /><span>{I18N.settings.textTranslationLabel} · {textTranslationDisplayName(profile)}</span></span>}
                  </span>
                  <span className="service-row__state">
                    <CredentialBadge state={credentialStateForTarget(profile, profile.languagePreset?.targetLanguage ?? settings.targetLanguage)} nativeSpeech={profile.provider === "appleSpeech"} />
                    {profile.id === settings.activeProfileId && (
                      <span className="profile-active-badge">
                        {I18N.settings.activeProfile}
                      </span>
                    )}
                  </span>
                </button>
                <button
                  type="button"
                  className="service-row__edit"
                  disabled={pendingAction !== null}
                  aria-label={`${I18N.settings.editProfile}: ${profile.name}`}
                  onClick={() => openEditor(profile.id)}
                >
                  <Icon name="chevron-right" />
                </button>
              </div>
            ))}
          </div>
          {atProfileLimit && (
            <InlineFeedback tone="info">{I18N.settings.profileLimitReached}</InlineFeedback>
          )}
        </div>
      )}
      {feedback && !showsProviderPicker && !(showsEditor && selectedProfile) && <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>}
    </SettingsSection>
    </>
  );
}

function CredentialEditor({
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
  readOnly = false,
}: {
  connectionCheck?: ReactNode | ((draft?: ProviderCredentialsInput | null) => ReactNode);
  textConnectionCheck?: (draft?: ProviderCredentialsInput | null) => ReactNode;
  readOnly?: boolean;
  profile: ServiceProfile;
  inputId: string;
  disabled: boolean;
  busy: boolean;
  visible: boolean;
  feedback: Feedback | null;
  onSave: (credentials: ProviderCredentialsInput) => Promise<unknown>;
  onRequestDelete: () => void;
  onConfirmDelete: () => Promise<unknown>;
  confirmingDelete: boolean;
  onCancelDelete: () => void;
}) {
  const [draft, setDraft] = useState<CredentialDraft>(emptyCredentialDraft);
  const [editingSavedCredential, setEditingSavedCredential] = useState(false);
  const [changedFields, setChangedFields] = useState<Partial<Record<CredentialFieldName, true>>>({});
  const [editorEpoch, setEditorEpoch] = useState(0);
  const editorState = useCredentialEditorState(profile.id, undefined, !readOnly && visible && (profile.credentialState !== "present" || editingSavedCredential), editorEpoch);
  const savedValues = editorState.state;
  const displayedDraft = { ...draft };
  for (const field of ["endpoint", "model", "deployment", "transcriptionDeployment", "appId"] as const) {
    if (!changedFields[field]) displayedDraft[field] = savedValues?.[field] ?? "";
  }
  const [endpointInvalid, setEndpointInvalid] = useState(false);
  const endpointRef = useRef<ConfigInputElement>(null);
  const feedbackRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (endpointInvalid) {
      endpointRef.current?.focus({ preventScroll: true });
      endpointRef.current?.closest("label")?.scrollIntoView({ block: "center" });
    }
  }, [endpointInvalid]);
  useEffect(() => {
    if (feedback?.tone === "error") {
      feedbackRef.current?.scrollIntoView({ block: "center" });
      feedbackRef.current?.focus({ preventScroll: true });
    }
  }, [feedback]);
  const saveFeedback = feedback && <div ref={feedbackRef} tabIndex={-1}><InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback></div>;
  const noteId = `${inputId}-storage-note`;
  const credentials = profile.credentialState === "present" && (editorState.loading || editorState.error) ? null : buildProviderCredentials(profile.provider, displayedDraft);

  const hasCredentialChanges = Object.keys(changedFields).length > 0;
  // A complete explicit replacement can be checked without saved metadata.
  // Partial drafts may reuse saved fields only after that metadata loaded.
  const probeCredentials = buildProviderCredentials(profile.provider, draft)
    ?? (editorState.loading || editorState.error ? null
      : buildProviderProbeCredentials(profile.provider, displayedDraft, savedValues?.savedFields));
  const check = typeof connectionCheck === "function"
    ? connectionCheck(readOnly || !hasCredentialChanges ? undefined : probeCredentials)
    : connectionCheck;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!credentials || disabled || readOnly) return;
    if (profile.provider === "deepLX" && !deepLXEndpointIsValid(displayedDraft.endpoint)) {
      setEndpointInvalid(true);
      endpointRef.current?.focus();
      return;
    }
    setEndpointInvalid(false);
    setEditingSavedCredential(true);
    // Reading a saved value does not create a replacement draft. Keep only
    // the user's edits on failure; discard after a successful save/use.
    void onSave(credentials).then((saved) => {
      if (saved) {
        setDraft(emptyCredentialDraft());
        setChangedFields({});
        setEditorEpoch(current => current + 1);
        setEditingSavedCredential(false);
      }
    });
  };

  const handleConfirmDelete = () => {
    const next = credentialEditorStateAfterDeleteRequest({ draft, editingSavedCredential }, true);
    // Clear plaintext before the async keychain deletion starts. A failure
    // must never restore a replacement secret to WebView state or the DOM.
    setDraft(next.draft);
    setChangedFields({});
    setEditorEpoch(current => current + 1);
    setEditingSavedCredential(next.editingSavedCredential);
    void onConfirmDelete();
  };

  const serviceIdentity = <section className="service-stage service-stage--integrated">
    <header className="service-stage__heading"><h3>{I18N.settings.voiceTranslation}</h3><SettingsHelp text={I18N.settings.textTranslationUnsupported} label={I18N.settings.helpLabel} /></header>
    <div className="settings-field service-stage__selector"><span>{I18N.settings.serviceProvider}</span><span className="service-stage__provider"><ProviderIcon provider={profile.provider} size={32} />{providerDisplayName(profile.provider)}</span></div>
    {profile.provider === "tencentCloud" && <TencentSetupHelp />}
  </section>;
  const storageHelp = <CredentialStorageHelp id={noteId} profile={profile} readOnly={readOnly} />;
  if (readOnly) return <div className="credential-panel"><div className="service-credential-toolbar">{storageHelp}{check}</div>{serviceIdentity}</div>;

  if (profile.credentialState === "present" && !editingSavedCredential) {
    return (
      <div className="credential-panel credential-panel--saved">
        <div className="service-credential-toolbar">{storageHelp}{check}<span className="credential-panel__saved-actions">
          <button
            type="button"
            className="settings-button settings-button--quiet settings-button--compact"
            disabled={disabled}
            onClick={() => setEditingSavedCredential(true)}
          >
            <Icon name="key" />{I18N.settings.replaceCredentials}
          </button>
          <button
            type="button"
            className="settings-button settings-button--quiet settings-button--compact"
            disabled={disabled || confirmingDelete}
            onClick={onRequestDelete}
          >
            <Icon name="trash" />{I18N.settings.deleteCredentials}
          </button>
        </span></div>
        {serviceIdentity}
        {saveFeedback}
        {confirmingDelete && (
          <DestructiveConfirmation
            message={I18N.settings.deleteCredentialsConfirm}
            disabled={disabled}
            onCancel={onCancelDelete}
            onConfirm={handleConfirmDelete}
          />
        )}
      </div>
    );
  }

  return (
    <div className="credential-panel" aria-busy={busy}>
      <div className="service-credential-toolbar">{check}</div>
      {serviceIdentity}
      <div className="credential-panel__heading">
        {storageHelp}
        {profile.credentialState === "present" && (
          <button
            type="button"
            className="settings-button settings-button--quiet settings-button--compact"
            disabled={disabled || confirmingDelete}
            onClick={onRequestDelete}
          >
            <Icon name="trash" />{I18N.settings.deleteCredentials}
          </button>
        )}
      </div>

      {profile.credentialState === "unavailable" && (feedback?.tone !== "error" || feedback.message === I18N.settings.profileActionFailed) && (
        <p className="credential-unavailable" role="status">
          {diagnosticCopy().storage}
        </p>
      )}

      {confirmingDelete && (
        <DestructiveConfirmation
          message={I18N.settings.deleteCredentialsConfirm}
          disabled={disabled}
          onCancel={onCancelDelete}
          onConfirm={handleConfirmDelete}
        />
      )}

      <form className="credential-form" onSubmit={handleSubmit}>
        {editorState.error && <InlineFeedback tone="error">{editorState.error}<button type="button" className="settings-link" disabled={disabled} onClick={() => setEditorEpoch(current => current + 1)}>{I18N.settings.retryLoadingSettings}</button></InlineFeedback>}
        <div className="credential-form__fields">
          {credentialFieldsForProvider(profile.provider).map((field) => {
            const copy = credentialFieldCopy(field, profile.provider);
            const fieldId = `${inputId}-${field}`;
            return (
              <div className="settings-field" key={field}>
                <label htmlFor={fieldId}>{copy.label}</label>
                {copy.secret ? <SavedCredentialInput
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
                /> : <ConfigInput expandable
                  id={fieldId}
                  ref={profile.provider === "deepLX" && field === "endpoint" ? endpointRef : undefined}
                  aria-invalid={field === "endpoint" && endpointInvalid ? true : undefined}
                  type="text"
                  inputMode={field === "appId" ? "numeric" : undefined}
                  value={displayedDraft[field]}
                  autoComplete="off"
                  spellCheck={false}
                  aria-describedby={field === "endpoint" && endpointInvalid ? `${fieldId}-error ${noteId}` : noteId}
                  disabled={disabled}
                  placeholder={copy.placeholder}
                  onValueChange={(value) => {
                    setEditingSavedCredential(true);
                    setChangedFields((current) => ({ ...current, [field]: true }));
                    setDraft((current) => ({ ...current, [field]: value }));
                    if (field === "endpoint" && endpointInvalid) setEndpointInvalid(!deepLXEndpointIsValid(value));
                  }}
                />}
                {field === "endpoint" && endpointInvalid && <span id={`${fieldId}-error`} role="alert" className="credential-unavailable">{I18N.settings.deepLXEndpointInvalid}</span>}
              </div>
            );
          })}
        </div>
        {saveFeedback}
        <span className="credential-form__actions">
          {profile.credentialState === "present" && (
            <button
              type="button"
              className="settings-button settings-button--quiet settings-button--compact"
              disabled={disabled}
              onClick={() => {
                setDraft(emptyCredentialDraft());
                setChangedFields({});
                setEditingSavedCredential(false);
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
            <Icon name="key" />{profile.credentialState === "present"
              ? I18N.settings.replaceCredentials
              : I18N.settings.saveAndUse}
          </button>
        </span>
      </form>
    </div>
  );
}

function profileProviderName(profile: ServiceProfile): string {
  return providerDisplayName(profile.provider === "deepLX" ? "alibabaCloud" : profile.provider);
}

function profileTitle(profile: ServiceProfile): string {
  return profile.provider === "deepLX" && profile.name === "DeepLX (Audio 3.0 ASR)" ? profileProviderName(profile) : profile.name;
}

function profileDescription(profile: ServiceProfile): string {
  if (profile.provider === "appleSpeech") return I18N.settings.appleSpeechDescription;
  if (isCustomSpeechProvider(profile.provider)) return [I18N.settings.customSpeechDescription, I18N.settings.customSpeechLanguages].join("\n");
  const translation = textTranslationForProfile(profile);
  return translation === "deepL" ? I18N.settings.deepLChain : translation === "deepLX" ? I18N.settings.deepLXChain : (translation === "openAICompatible" || translation === "chatMock") ? I18N.settings.openAICompatibleChain : providerDescription(profile.provider);
}

function profileSecondaryLabel(profile: ServiceProfile): string | null {
  if (textTranslationForProfile(profile) !== "followService") return `${I18N.settings.speechRecognition} · ${speechRecognitionDisplayName(profile)}`;
  const provider = speechRecognitionDisplayName(profile);
  return profileTitle(profile).trim().toLowerCase() === provider.toLowerCase() ? null : provider;
}

function credentialFieldCopy(field: CredentialFieldName, provider: ServiceProvider): {
  label: string;
  placeholder: string;
  secret: boolean;
} {
  switch (field) {
    case "model":
      return { label: I18N.settings.customSpeechModel, placeholder: I18N.settings.modelNamePlaceholder, secret: false };
    case "asrApiKey":
      return { label: I18N.settings.asrApiKey, placeholder: I18N.settings.apiKeyPlaceholder, secret: true };
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
        placeholder: provider === "deepLX" ? I18N.settings.serviceAddressPlaceholder : I18N.settings.azureEndpointPlaceholder,
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

function ProviderPicker({
  apple,
  disabled,
  cancelDisabled,
  requiresStop,
  feedback,
  onChoose,
  onCancel,
}: {
  apple: ReturnType<typeof useAppleSpeechSupport>;
  disabled: boolean;
  cancelDisabled: boolean;
  requiresStop: boolean;
  feedback: Feedback | null;
  onChoose: (provider: ServiceProvider) => void;
  onCancel: () => void;
}) {
  const [selectedProvider, setSelectedProvider] = useState<ServiceProvider | null>(null);
  return (
    <div className="provider-picker settings-panel">
      <div className="provider-picker__heading">
        <span>
          <strong>{I18N.settings.chooseProvider}</strong>
          <SettingsHelp text={I18N.settings.chooseProviderDescription} label={I18N.settings.helpLabel} />
        </span>
        <button type="button" className="settings-link" disabled={cancelDisabled} onClick={onCancel}>{I18N.settings.cancel}</button>
      </div>
      {!selectedProvider && requiresStop && <InlineFeedback tone="info">{I18N.settings.profileCreateRequiresStop}</InlineFeedback>}
      {!selectedProvider && feedback && <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>}
      {apple.failed && <InlineFeedback tone="error">{I18N.settings.appleSpeechLoadFailed} <button type="button" className="settings-link" disabled={disabled || apple.loading} onClick={() => void apple.refresh()}>{I18N.settings.retryLoadingSettings}</button></InlineFeedback>}
      <div className="provider-picker__options">
        {SERVICE_PROVIDERS.filter(provider => provider !== "appleSpeech" || apple.support?.available).map((provider) => (
          <div className="provider-picker__option-row" key={provider}>
            <button type="button" className="provider-option" data-provider={provider} disabled={disabled} onClick={() => setSelectedProvider(provider)}>
              <ProviderIcon provider={provider} />
              <span><strong>{providerDisplayName(provider)}</strong></span>
              <Icon name="chevron-right" />
            </button>
            <SettingsHelp text={providerDescription(provider)} label={`${providerDisplayName(provider)}: ${I18N.settings.helpLabel}`} />
          </div>
        ))}
      </div>
      {selectedProvider && <SettingsConfirmation message={I18N.settings.confirmAddProfile} disabled={disabled} variant="default" confirmLabel={I18N.settings.confirmAddProfile} confirmIcon="plus" onCancel={() => { if (!cancelDisabled) setSelectedProvider(null); }} onConfirm={() => onChoose(selectedProvider)}>
        <div className="provider-picker__preview">
          <ProviderIcon provider={selectedProvider} />
          <div className="provider-picker__name-help"><h3>{providerDisplayName(selectedProvider)}</h3><SettingsHelp text={providerDescription(selectedProvider)} label={I18N.settings.helpLabel} /></div>
        </div>
        {requiresStop && <InlineFeedback tone="info">{I18N.settings.profileCreateRequiresStop}</InlineFeedback>}
        {feedback && <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>}
      </SettingsConfirmation>}
    </div>
  );
}

function CredentialBadge({ state, nativeSpeech = false }: { state: CredentialState; nativeSpeech?: boolean }) {
  const label = credentialStateText(state, nativeSpeech);
  return <Tooltip label={label} popupClassName="settings-help-tooltip">{(descriptionId) => <span className="credential-badge" data-state={state} role="img" aria-label={label} aria-describedby={descriptionId} tabIndex={0}><Icon name={state === "present" ? nativeSpeech ? "checkmark-circle" : "shield-check" : state === "missing" && !nativeSpeech ? "key" : "exclamation-triangle"} /></span>}</Tooltip>;
}

function providerDescription(provider: ServiceProvider): string {
  switch (provider) {
    case "appleSpeech":
      return I18N.settings.appleSpeechDescription;
    case "customDashScopeASR":
      return [I18N.settings.customSpeechRequirementsDashScope, I18N.settings.customSpeechLanguages].join("\n");
    case "customOpenAIASR":
      return [I18N.settings.customSpeechRequirementsOpenAI, I18N.settings.customSpeechLanguages].join("\n");
    case "alibabaCloud":
      return I18N.settings.providerAlibabaDescription;
    case "openAIRealtime":
      return I18N.settings.providerOpenAIDescription;
    case "googleGeminiLive":
      return I18N.settings.providerGoogleGeminiDescription;
    case "azureOpenAIRealtime":
      return I18N.settings.providerAzureOpenAIDescription;
    case "volcanoEngine":
      return I18N.settings.providerVolcanoEngineDescription;
    case "tencentCloud":
      return I18N.settings.providerTencentCloudDescription;
    case "baiduTranslate":
      return I18N.settings.providerBaiduTranslateDescription;
    case "deepLX":
      return I18N.settings.providerDeepLXDescription;
    case "xAIRealtime":
      return I18N.settings.providerXAIDescription;
  }
}

function credentialStateText(state: CredentialState, nativeSpeech = false): string {
  if (nativeSpeech) return state === "present" ? I18N.settings.appleSpeechServiceReady : state === "missing" ? I18N.settings.appleSpeechNeedsSetup : I18N.settings.appleSpeechUnavailable;
  switch (state) {
    case "present":
      return I18N.settings.credentialPresent;
    case "missing":
      return I18N.settings.credentialMissing;
    case "unavailable":
      return I18N.settings.credentialUnavailable;
  }
}

function focusAppleSpeechResources(): boolean {
  const resources = document.getElementById("apple-speech-resources");
  if (!resources) return false;
  resources.scrollIntoView({ block: "start" });
  resources.focus({ preventScroll: true });
  return true;
}
