import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { I18N, providerDisplayName } from "../../../lib/i18n";
import type { ConnectionCheckStage } from "../../../lib/ipc";
import { profileCopyName } from "../../../lib/profiles/profileCopyName";
import { profileSelectionChangesSettings } from "../../../lib/profiles/profileLanguagePreset";
import { profileErrorMessage } from "../../../lib/providers/connectionDiagnostics";
import {
  credentialStateForTarget,
  isIndependentAsrProfile,
  subtitlePreferencesChanged,
  textTranslationForProfile,
  usesAlibabaRealtime,
} from "../../../lib/providers/providerCapabilities";
import { speechLanguageGuidance } from "../../../lib/providers/speechLanguageGuidance";
import { useStore } from "../../../lib/store";
import type {
  AlibabaRealtimeMode,
  NetworkProxyConfig,
  ProfileLanguagePreset,
  ProfileOptionsDraft,
  ProviderCredentialsInput,
  QwenMTModel,
  ServiceProfile,
  ServiceProvider,
  SessionStateEvent,
  SettingsSnapshot,
  SourceLanguage,
  TextTranslationNameDraft,
} from "../../../lib/types";
import { SettingsConfirmation } from "../controls/SettingsConfirmation";
import { InlineFeedback, SettingsSection } from "../controls/SettingsPrimitives";
import { SettingsInitializationStatus } from "../feedback/SettingsInitializationStatus";
import { dismissSettingsToast, useSettingsToast } from "../feedback/useSettingsToast";
import { DraftConnectionCheck } from "./connection/ConnectionCheck";
import { useProfileConnectionChecks } from "./connection/useProfileConnectionChecks";
import { NameDraftContext, NameDraftOwner } from "./profiles/nameDraftOwner";
import { ProviderPicker } from "./profiles/ProviderPicker";
import { saveAndSelectProfile } from "./profiles/saveAndSelectProfile";
import { useServicePageNavigation } from "./profiles/useServicePageNavigation";
import { useAppleSpeechSupport } from "./providers/apple/useAppleSpeechSupport";
import { ServiceProfileEditor, type ServiceProfileFeedback } from "./ServiceProfileEditor";
import { ServiceProfileList } from "./ServiceProfileList";

type PendingAction = "duplicate" | "create" | "select" | "delete" | "save-key" | "save-model" | "delete-key" | "test-connection" | "save-proxy" | "save-languages" | "switch-language" | "prepare-resource" | "prepare-apple-translation" | null;
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
  const duplicateProfile = useStore((state) => state.duplicateProfile);
  const createProfile = useStore((state) => state.createProfile);
  const updateProfile = useStore((state) => state.updateProfile);
  const selectProfile = useStore((state) => state.selectProfile);
  const deleteProfile = useStore((state) => state.deleteProfile);
  const saveProfileCredentials = useStore((state) => state.saveProfileCredentials);
  const deleteProfileAPIKey = useStore((state) => state.deleteProfileAPIKey);
  const initializationStatus = useStore((state) => state.initializationStatus);
  const initializationError = useStore((state) => state.initializationError);
  const initialize = useStore((state) => state.init);
  const apple = useAppleSpeechSupport(visible);
  const canUseProfile = (profile: ServiceProfile) => {
    if (usesAlibabaRealtime(profile) && !profile.alibabaWorkspaceId?.trim()) return false;
    return profile.provider === "appleSpeech"
      ? apple.support?.available === true
      : credentialStateForTarget(profile, profile.languagePreset?.targetLanguage ?? settings.targetLanguage) === "present";
  };

  const activeProfile =
    settings.profiles.find((profile) => profile.id === settings.activeProfileId) ??
    settings.profiles[0];
  const [selectedProfileId, setSelectedProfileId] = useState(
    activeProfile?.id ?? settings.activeProfileId,
  );
  const [nameDraftOwner] = useState(() => new NameDraftOwner());
  // A settings event can delete a profile before its editor's passive cleanup.
  // Cancel that owner's pending flush before the detached field can replay it.
  useLayoutEffect(() => nameDraftOwner.prune(settings.profiles), [nameDraftOwner, settings.profiles]);
  useEffect(() => nameDraftOwner.sync(settings.profiles), [nameDraftOwner, settings.profiles]);
  const [showsEditor, setShowsEditor] = useState(settings.profiles.length === 1);
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
  const [feedback, setFeedback] = useState<ServiceProfileFeedback | null>(null);
  const { beginToast } = useSettingsToast();
  const creationInFlight = useRef(false);
  const mutationInFlight = useRef(false);
  const [pendingConfirmation, setPendingConfirmation] = useState<PendingConfirmation>(null);
  const mounted = useRef(false);
  const [renderedProfileId, setRenderedProfileId] = useState(activeProfile?.id);

  const selectedProfile = useMemo(
    () => settings.profiles.find((profile) => profile.id === selectedProfileId) ?? activeProfile,
    [activeProfile, selectedProfileId, settings.profiles],
  );

  const { diagnostics, pendingCheckStage, invalidateProfileCheck, handleConnectionCheck } = useProfileConnectionChecks({
    settings, sessionStatusKind, selectedProfile, actionPending: pendingAction !== null, setPendingAction,
  });
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  // A lone configuration is the settings page, rather than an empty catalog.
  // Keep an already-open editor mounted if another configuration arrives.
  const singleProfile = settings.profiles.length === 1;
  if (singleProfile && !showsEditor && !showsProviderPicker) setShowsEditor(true);

  const navigationRoot = useRef<HTMLDivElement>(null);
  const initialSetup = settings.profiles.length === 0;
  const pickerVisible = showsProviderPicker || initialSetup;
  const pageKey = pickerVisible ? "picker" : showsEditor && selectedProfile ? `editor:${selectedProfile.id}` : "list";
  const rememberPagePosition = useServicePageNavigation(navigationRoot, pageKey, visible, openedProfileEditorRequest);

  if (selectedProfile && selectedProfile.id !== renderedProfileId) {
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

  const requiresStop = sessionIsActive || sessionIsPaused || sessionStatusKind === "connecting" || sessionStatusKind === "stopping";
  const mutationsDisabled = requiresStop || pendingAction !== null;
  const selectionDisabled = sessionStatusKind === "connecting" || sessionStatusKind === "stopping" || pendingAction !== null;
  const atProfileLimit = settings.profiles.length >= 20;

  const trackProfileOperation = async (operation: () => Promise<SettingsSnapshot>) => {
    setProfileOperationFailed(false);
    try {
      return await operation();
    } catch (error) {
      if (mounted.current) setProfileOperationFailed(true);
      throw error;
    }
  };

  const runProfileMutation = async (
    action: Exclude<PendingAction, null>,
    operation: () => Promise<SettingsSnapshot>,
    options?: {
      blockedError?: string;
      invalidate?: { profileId: string; stage?: ConnectionCheckStage };
      onSuccess?: (snapshot: SettingsSnapshot) => void;
    },
  ): Promise<SettingsSnapshot> => {
    if (mutationInFlight.current || mutationsDisabled) {
      throw new Error(options?.blockedError ?? "profile-change-requires-stop");
    }
    mutationInFlight.current = true;
    setPendingAction(action);
    try {
      const snapshot = await trackProfileOperation(operation);
      if (options?.invalidate) {
        invalidateProfileCheck(options.invalidate.profileId, options.invalidate.stage);
      }
      options?.onSuccess?.(snapshot);
      return snapshot;
    } finally {
      mutationInFlight.current = false;
      if (mounted.current) setPendingAction(null);
    }
  };

  const perform = async (
    action: Exclude<PendingAction, null>,
    operation: () => Promise<SettingsSnapshot>,
    successFeedback: string | ((snapshot: SettingsSnapshot) => ServiceProfileFeedback),
  ): Promise<SettingsSnapshot | null> => {
    if (mutationInFlight.current) return null;
    mutationInFlight.current = true;
    if (action === "delete-key") invalidateProfileCheck(selectedProfileId);
    setPendingAction(action);
    setFeedback(null);
    const notify = beginToast();
    try {
      const snapshot = await trackProfileOperation(operation);
      const result: ServiceProfileFeedback = typeof successFeedback === "string"
        ? { tone: "success", message: successFeedback }
        : successFeedback(snapshot);
      if (result.tone !== "error") notify(result.message);
      else setFeedback(result);
      return snapshot;
    } catch (error) {
      // Keep unsaved edits actionable in their form; list operations have no field to correct.
      if (action === "save-key") setFeedback({ tone: "error", message: profileErrorMessage(error) });
      else notify(profileErrorMessage(error, action === "duplicate" ? I18N.settings.profileDuplicateFailed : undefined), true);
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
      if (!snapshot || !mounted.current) return;
      // Creation appends its own profile last in the command acknowledgment.
      // Earlier additions may have arrived while this request was waiting.
      const created = snapshot.profiles.at(-1);
      if (created && !previousIds.has(created.id)) {
        setSelectedProfileId(created.id);
        setShowsEditor(true);
      }
      setShowsProviderPicker(false);
    } finally {
      creationInFlight.current = false;
    }
  };

  const handleDuplicate = async (profile: ServiceProfile) => {
    if (mutationsDisabled || atProfileLimit) return;
    if (pageKey === "list") rememberPagePosition(`profile-more-${profile.id}`);
    const previousIds = new Set(settings.profiles.map(item => item.id));
    const name = profileCopyName(profile.name, I18N.settings.profileCopySuffix, settings.profiles);
    const snapshot = await perform("duplicate", () => duplicateProfile(profile.id, name), I18N.settings.profileDuplicated);
    if (!snapshot || !mounted.current) return;
    // The command appends its own copy last and the store returns that exact
    // response, even if another window published newer settings meanwhile.
    const copy = snapshot.profiles.at(-1);
    if (copy && !previousIds.has(copy.id)) { setSelectedProfileId(copy.id); setShowsEditor(true); }
  };

  const handleRename = (profileId: string, name: string) => updateProfile(profileId, name);

  const handleSaveTranslationName = (profile: ServiceProfile, route: TextTranslationNameDraft["route"], name: string) =>
    updateProfile(profile.id, undefined, { textTranslationName: { route, name } });

  const handleSaveRecognitionName = (profileId: string, name: string) =>
    updateProfile(profileId, undefined, { speechRecognitionName: name });

  const handleSaveAlibabaRealtime = async (
    profileId: string,
    mode: AlibabaRealtimeMode,
    workspaceId: string,
    savedMessage: string,
  ) => {
    const notify = beginToast();
    await runProfileMutation(
      "save-model",
      () => updateProfile(profileId, undefined, { alibabaRealtimeMode: mode, alibabaWorkspaceId: workspaceId }),
      {
        invalidate: { profileId },
        onSuccess: () => notify(savedMessage),
      },
    );
  };

  const handleSaveQwenMtModel = async (profileId: string, model: QwenMTModel) => {
    await runProfileMutation(
      "save-model",
      () => updateProfile(profileId, undefined, { qwenMtModel: model }),
      { invalidate: { profileId, stage: "text" } },
    );
  };

  const handleSaveSpeechOptions = (profileId: string, options: ProfileOptionsDraft) =>
    runProfileMutation(
      "save-model",
      () => updateProfile(profileId, undefined, options),
      { invalidate: { profileId } },
    );

  const handleSaveProxy = async (profile: ServiceProfile, stage: "speech" | "text", config: NetworkProxyConfig) => {
    await runProfileMutation(
      "save-proxy",
      () => updateProfile(profile.id, undefined, stage === "speech" ? { speechNetworkProxy: config } : { textNetworkProxy: config }),
      { blockedError: "network_proxy_change_requires_stop", invalidate: { profileId: profile.id, stage } },
    );
  };

  const handleSaveLanguagePreset = (profileId: string, preset: ProfileLanguagePreset | null) =>
    runProfileMutation("save-languages", () => updateProfile(profileId, undefined, { languagePreset: preset }));

  const handleSaveSpeechLanguages = (profile: ServiceProfile, languages: SourceLanguage[] | null) => {
    const before = settings;
    const notify = beginToast();
    // The command response acknowledges this exact save even if its settings
    // event arrives first. Never infer normalization from an unrelated event.
    return runProfileMutation(
      "save-languages",
      () => updateProfile(profile.id, undefined, { customSpeechSourceLanguages: languages }),
      {
        onSuccess: (after) => {
          if (!mounted.current) return;
          invalidateProfileCheck(profile.id);
          notify(
            profile.id === before.activeProfileId && after.activeProfileId === before.activeProfileId && after.sourceLanguage !== before.sourceLanguage
              ? I18N.settings.recognitionLanguageAdjusted(
                speechLanguageGuidance(before).optionLabel(before.sourceLanguage),
                speechLanguageGuidance(after).optionLabel(after.sourceLanguage),
              )
              : I18N.settings.customSpeechLanguagesSaved,
          );
        },
      },
    );
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
            ? { tone: "info", message: I18N.settings.profileSelectedWithAdjustments }
            : { tone: "success", message: I18N.settings.profileSelected },
    );
  };

  const handleSelectAppleProfile = async (profileId: string, sourceLanguage: SourceLanguage) => {
    if (selectionDisabled || mutationInFlight.current) throw new Error("profile_switch_busy");
    mutationInFlight.current = true;
    try {
      await selectProfile(profileId, sourceLanguage);
      invalidateProfileCheck(profileId);
    } finally {
      mutationInFlight.current = false;
    }
  };

  const requestProfileDelete = (profile: ServiceProfile) => {
    if (mutationsDisabled || mutationInFlight.current || settings.profiles.length <= 1) return;
    setPendingConfirmation({ kind: "profile", profileId: profile.id, name: profile.name });
  };

  const confirmProfileDelete = async () => {
    if (
      mutationsDisabled ||
      settings.profiles.length <= 1 ||
      pendingConfirmation?.kind !== "profile" ||
      !settings.profiles.some(profile => profile.id === pendingConfirmation.profileId)
    )
      return;
    setPendingConfirmation(null);
    const snapshot = await perform(
      "delete",
      () => deleteProfile(pendingConfirmation.profileId),
      I18N.settings.profileDeleted,
    );
    if (snapshot) {
      nameDraftOwner.discard(pendingConfirmation.profileId);
      setSelectedProfileId(snapshot.activeProfileId);
      setShowsEditor(false);
    }
  };

  const handleSaveCredential = async (profileId: string, credentials: ProviderCredentialsInput) => {
    setPendingConfirmation(null);
    let stage: ConnectionCheckStage | undefined;
    if (credentials.kind === "customSpeech") {
      stage = "speech";
    } else if ((credentials.kind === "apiKey" || credentials.kind === "azureOpenAI")
      && selectedProfile && isIndependentAsrProfile(selectedProfile)) {
      stage = "speech";
    } else if (credentials.kind === "alibabaTranslation"
      && (isIndependentAsrProfile(selectedProfile!) || !credentials.apiKey.trim())) {
      stage = "text";
    }
    invalidateProfileCheck(profileId, stage);
    return perform(
      "save-key",
      () => selectedProfile?.id === profileId && selectedProfile.id !== settings.activeProfileId
        && (selectedProfile.credentialState === "present" || selectedProfile.speechCredentialState === "present")
        ? saveProfileCredentials(profileId, credentials)
        : saveAndSelectProfile(profileId, credentials, saveProfileCredentials, selectProfile),
      (snapshot) => {
        const preferencesAdjusted = subtitlePreferencesChanged(settings, snapshot);
        const localTextConfiguration = credentials.kind === "alibabaTranslation"
          && credentials.textTranslation === "apple" && !credentials.apiKey.trim();
        return {
          tone: preferencesAdjusted ? "info" : "success",
          message: preferencesAdjusted ? I18N.settings.profileSelectedWithAdjustments
            : localTextConfiguration ? I18N.settings.translationConfigurationSaved
            : I18N.settings.credentialsSaved,
        };
      },
    );
  };

  const requestCredentialDelete = (profileId: string) => {
    setPendingConfirmation({ kind: "credential", profileId });
  };

  const confirmCredentialDelete = async (profileId: string) => {
    if (pendingConfirmation?.kind !== "credential" || pendingConfirmation.profileId !== profileId)
      return;
    setPendingConfirmation(null);
    await perform("delete-key", () => deleteProfileAPIKey(profileId), I18N.settings.credentialsDeleted);
  };

  const openProviderPicker = () => {
    dismissSettingsToast();
    rememberPagePosition("service-add-profile");
    setFeedback(null);
    setShowsProviderPicker(true);
  };

  const openEditor = (profileId: string) => {
    dismissSettingsToast();
    rememberPagePosition(`profile-edit-${profileId}`);
    setSelectedProfileId(profileId);
    setPendingConfirmation(null);
    setFeedback(null);
    setShowsEditor(true);
  };

  const closeEditor = () => {
    dismissSettingsToast();
    setShowsEditor(false);
    setPendingConfirmation(null);
    setFeedback(null);
  };

  const renderConnectionCheck = (profile: ServiceProfile, stage?: ConnectionCheckStage, draft?: ProviderCredentialsInput | null, sourceLanguage?: SourceLanguage) => {
    const key = stage ?? "combined";
    const outcome = diagnostics[key];
    const context = JSON.stringify([profile.provider, textTranslationForProfile(profile), profile.alibabaRealtimeMode ?? "disabled", profile.alibabaWorkspaceId ?? "", profile.xaiSpeechMode ?? "voice", profile.speechTranslationMode ?? "simultaneous", sourceLanguage ?? settings.sourceLanguage,
      stage === "speech" ? null : settings.targetLanguage]);
    return (
      <DraftConnectionCheck
        key={`${profile.id}:${key}`}
        draft={draft}
        context={context}
        outcome={outcome?.profileId === profile.id ? outcome : undefined}
        pending={pendingCheckStage === key}
        disabled={mutationsDisabled || (usesAlibabaRealtime(profile) && !profile.alibabaWorkspaceId?.trim()) || (profile.provider === "appleSpeech" && stage === "speech" && apple.loading)}
        onCheck={input => { if (draft !== null) handleConnectionCheck(input, stage, draft, sourceLanguage); }}
        label={stage === "text" ? I18N.settings.checkTextTranslation : stage === "speech" ? I18N.settings.checkSpeechRecognition : undefined}
      />
    );
  };

  if (initializationStatus !== "ready") {
    return <SettingsInitializationStatus status={initializationStatus} error={initializationError} onRetry={() => { void initialize(); }} />;
  }

  return (
    <NameDraftContext.Provider value={{ owner: nameDraftOwner, profileId: selectedProfile?.id ?? "" }}>
      <div ref={navigationRoot}>
        {!showsEditor && !pickerVisible && overview}
        <SettingsSection id="service-profiles" title={I18N.settings.serviceProfilesTitle} hideHeading>
          {pickerVisible && (
            <ProviderPicker
              initialSetup={initialSetup}
              apple={apple}
              disabled={mutationsDisabled}
              cancelDisabled={pendingAction !== null}
              requiresStop={requiresStop}
              feedback={feedback}
              onChoose={(provider) => void handleCreate(provider)}
              onCancel={() => { dismissSettingsToast(); setShowsProviderPicker(false); }}
            />
          )}
          {showsEditor && selectedProfile ? (
            <ServiceProfileEditor
              profile={selectedProfile}
              settings={settings}
              apple={apple}
              hidden={pickerVisible}
              singleProfile={singleProfile}
              visible={visible && !pickerVisible && pendingConfirmation === null}
              requiresStop={requiresStop}
              actionPending={pendingAction !== null}
              credentialBusy={pendingAction === "save-key" || pendingAction === "delete-key"}
              duplicatePending={pendingAction === "duplicate"}
              mutationsDisabled={mutationsDisabled}
              selectionDisabled={selectionDisabled}
              atProfileLimit={atProfileLimit}
              canUseProfile={canUseProfile(selectedProfile)}
              confirmingCredentialDelete={
                pendingConfirmation?.kind === "credential" &&
                pendingConfirmation.profileId === selectedProfile.id
              }
              feedback={feedback}
              profileDetailHeading={profileDetailHeading}
              onBack={closeEditor}
              onOpenProviderPicker={openProviderPicker}
              onSelectProfile={(profileId) => void handleSelect(profileId)}
              onSelectAppleProfile={handleSelectAppleProfile}
              onRename={handleRename}
              onSaveTranslationName={handleSaveTranslationName}
              onSaveRecognitionName={handleSaveRecognitionName}
              onSaveAlibabaRealtime={handleSaveAlibabaRealtime}
              onSaveQwenMtModel={handleSaveQwenMtModel}
              onSaveSpeechOptions={handleSaveSpeechOptions}
              onSaveCredential={handleSaveCredential}
              onRequestCredentialDelete={requestCredentialDelete}
              onConfirmCredentialDelete={confirmCredentialDelete}
              onCancelCredentialDelete={() => setPendingConfirmation(null)}
              onSaveSpeechLanguages={handleSaveSpeechLanguages}
              onSaveLanguagePreset={handleSaveLanguagePreset}
              onSaveProxy={handleSaveProxy}
              onDuplicate={(profile) => void handleDuplicate(profile)}
              onRequestDelete={requestProfileDelete}
              onPrepareResourceBusyChange={(profileId, busy) => {
                if (busy) invalidateProfileCheck(profileId, "speech");
                setPendingAction(busy ? "prepare-resource" : null);
              }}
              onNativeTranslationBusyChange={(busy) => setPendingAction(busy ? "prepare-apple-translation" : null)}
              onNativeTranslationPrepared={(profileId) => invalidateProfileCheck(profileId, "text")}
              renderConnectionCheck={renderConnectionCheck}
            />
          ) : !pickerVisible && (
            <ServiceProfileList
              settings={settings}
              visible={visible}
              actionPending={pendingAction !== null}
              mutationsDisabled={mutationsDisabled}
              selectionDisabled={selectionDisabled}
              atProfileLimit={atProfileLimit}
              canUseProfile={canUseProfile}
              onOpenProviderPicker={openProviderPicker}
              onSelectProfile={(profileId) => void handleSelect(profileId)}
              onOpenEditor={openEditor}
              onDuplicateProfile={(profile) => void handleDuplicate(profile)}
              onRequestDeleteProfile={requestProfileDelete}
            />
          )}
          {feedback && !pickerVisible && !(showsEditor && selectedProfile) && (
            <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>
          )}
        </SettingsSection>
        {pendingConfirmation?.kind === "profile" && settings.profiles.some(profile => profile.id === pendingConfirmation.profileId) && (
          <SettingsConfirmation
            message={I18N.settings.deleteProfileConfirm(pendingConfirmation.name)}
            disabled={mutationsDisabled}
            onCancel={() => setPendingConfirmation(null)}
            onConfirm={() => void confirmProfileDelete()}
          />
        )}
      </div>
    </NameDraftContext.Provider>
  );
}

function focusAppleSpeechResources(): boolean {
  const resources = document.getElementById("apple-speech-resources");
  if (!resources) return false;
  resources.scrollIntoView({ block: "start" });
  resources.focus({ preventScroll: true });
  return true;
}
