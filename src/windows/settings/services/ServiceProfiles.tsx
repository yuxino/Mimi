import { ProviderCredentialEditor } from "./credentials/ProviderCredentialEditor";
import { ProviderPicker } from "./profiles/ProviderPicker";
import { profileTitle, profileDescription, profileSecondaryLabel, credentialStateText } from "./profiles/serviceProfilePresentation";
import { useProfileConnectionChecks } from "./connection/useProfileConnectionChecks";
import { profileSelectionChangesSettings } from "../../../lib/profiles/profileLanguagePreset";
import type { ConnectionCheckStage } from "../../../lib/ipc";
import { profileErrorMessage } from "../../../lib/providers/connectionDiagnostics";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Icon } from "../../../components/Icon";
import { ProviderIcon } from "../../../components/providers/ProviderIcon";
import { I18N, providerDisplayName } from "../../../lib/i18n";
import { credentialStateForTarget, isCustomSpeechProvider, isIndependentAsrProfile, subtitlePreferencesChanged, supportsSpeechTranslationMode, textTranslationForProfile, usesAlibabaRealtime, usesXaiAsr } from "../../../lib/providers/providerCapabilities";
import { DEFAULT_NETWORK_PROXY } from "../../../lib/providers/networkProxy";
import { useStore } from "../../../lib/store";
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
} from "../../../lib/types";
import { InlineFeedback, SettingsSection } from "../controls/SettingsPrimitives";
import { dismissSettingsToast, useSettingsToast } from "../feedback/useSettingsToast";

import { DestructiveConfirmation } from "../controls/DestructiveConfirmation";
import { AlibabaCredentialEditor } from "./providers/alibaba/AlibabaCredentialEditor";
import { CustomSpeechCredentialEditor } from "./credentials/CustomSpeechCredentialEditor";

import { SettingsActionMenu } from "../controls/SettingsActionMenu";
import { SettingsHelp } from "../feedback/SettingsHelp";
import { DraftConnectionCheck } from "./connection/ConnectionCheck";
import { saveAndSelectProfile } from "./profiles/saveAndSelectProfile";
import { SettingsInitializationStatus } from "../feedback/SettingsInitializationStatus";
import { textTranslationDisplayName } from "../../../lib/providers/textTranslationName";
import { speechRecognitionDisplayName } from "../../../lib/providers/speechRecognitionName";
import { NetworkProxySettings } from "./network/NetworkProxySettings";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES } from "../../../lib/types";
import { ProfileLanguagePresetSettings } from "./languages/ProfileLanguagePresetSettings";
import { CustomSpeechLanguageSettings } from "./languages/CustomSpeechLanguageSettings";
import { speechLanguageGuidance } from "../../../lib/providers/speechLanguageGuidance";
import { AutoSaveNameField } from "./profiles/AutoSaveNameField";
import { NameDraftContext, NameDraftOwner } from "./profiles/nameDraftOwner";
import { AppleSpeechSettings } from "./providers/apple/AppleSpeechSettings";
import { useAppleSpeechSupport } from "./providers/apple/useAppleSpeechSupport";
import { profileCopyName } from "../../../lib/profiles/profileCopyName";
import { AlibabaRealtimeSettings } from "./providers/alibaba/AlibabaRealtimeSettings";
import { QwenMTModelSettings } from "./providers/QwenMTModelSettings";
import { XAISpeechSettings } from "./providers/XAISpeechSettings";
import { SpeechTranslationModeSettings } from "./providers/SpeechTranslationModeSettings";
import { ServiceEditorModule } from "./ServiceEditorModule";
import { useServicePageNavigation } from "./profiles/useServicePageNavigation";

type Feedback = { tone: "success" | "error" | "info"; message: string };
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
  const initializationStatus = useStore((state) => state.initializationStatus) ?? "ready";
  const initializationError = useStore((state) => state.initializationError) ?? null;
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
  const [feedback, setFeedback] = useState<Feedback | null>(null);
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

  const hasIndependentTextProxy = !!selectedProfile && !usesAlibabaRealtime(selectedProfile) && (isIndependentAsrProfile(selectedProfile) || ["alibabaCloud", "deepLX"].includes(selectedProfile.provider));
  const hasRemoteTextProxy = hasIndependentTextProxy && selectedProfile && textTranslationForProfile(selectedProfile) !== "apple"
    && (!isIndependentAsrProfile(selectedProfile) || textTranslationForProfile(selectedProfile) !== "followService");

  const SelectedCredentialEditor = selectedProfile?.provider === "appleSpeech"
    ? AppleSpeechSettings
    : selectedProfile && isCustomSpeechProvider(selectedProfile.provider)
      ? CustomSpeechCredentialEditor
      : selectedProfile && ["alibabaCloud", "deepLX"].includes(selectedProfile.provider)
        ? AlibabaCredentialEditor
        : ProviderCredentialEditor;

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
    } finally { creationInFlight.current = false; }
  };

  const handleDuplicate = async (profile: ServiceProfile) => {
    if (mutationsDisabled || atProfileLimit) return;
    if (pageKey === "list") rememberPagePosition(`profile-more-${profile.id}`);
    const previousIds = new Set(settings.profiles.map(profile => profile.id));
    const name = profileCopyName(profile.name, I18N.settings.profileCopySuffix, settings.profiles);
    const snapshot = await perform("duplicate", () => duplicateProfile(profile.id, name), I18N.settings.profileDuplicated);
    if (!snapshot || !mounted.current) return;
    // The command appends its own copy last and the store returns that exact
    // response, even if another window published newer settings meanwhile.
    const copy = snapshot.profiles.at(-1);
    if (copy && !previousIds.has(copy.id)) { setSelectedProfileId(copy.id); setShowsEditor(true); }
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
    if (!selectedProfile || mutationInFlight.current || mutationsDisabled) throw new Error("profile-change-requires-stop");
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
    if (mutationInFlight.current || mutationsDisabled) throw new Error("profile-change-requires-stop");
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

  const requestProfileDelete = (profile: ServiceProfile) => {
    if (mutationsDisabled || mutationInFlight.current || settings.profiles.length <= 1) return;
    setPendingConfirmation({
      kind: "profile",
      profileId: profile.id,
      name: profile.name,
    });
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
    await perform(
      "delete-key",
      () => deleteProfileAPIKey(profileId),
      I18N.settings.credentialsDeleted,
    );
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

  const renderConnectionCheck = (profile: ServiceProfile, stage?: ConnectionCheckStage, draft?: ProviderCredentialsInput | null, sourceLanguage?: SourceLanguage) => {
    const key = stage ?? "combined";
    const outcome = diagnostics[key];
    const context = JSON.stringify([profile.provider, textTranslationForProfile(profile), profile.alibabaRealtimeMode ?? "disabled", profile.alibabaWorkspaceId ?? "", profile.xaiSpeechMode ?? "voice", profile.speechTranslationMode ?? "simultaneous", sourceLanguage ?? settings.sourceLanguage,
      stage === "speech" ? null : settings.targetLanguage]);
    return <DraftConnectionCheck key={`${profile.id}:${key}`} draft={draft} context={context}
      outcome={outcome?.profileId === profile.id ? outcome : undefined}
      pending={pendingCheckStage === key} disabled={mutationsDisabled || (usesAlibabaRealtime(profile) && !profile.alibabaWorkspaceId?.trim()) || (profile.provider === "appleSpeech" && stage === "speech" && apple.loading)}
      onCheck={input => { if (draft !== null) handleConnectionCheck(input, stage, draft, sourceLanguage); }}
      label={stage === "text" ? I18N.settings.checkTextTranslation : stage === "speech" ? I18N.settings.checkSpeechRecognition : undefined} />;
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
        <div className="service-detail" hidden={pickerVisible}>
          <div className="service-detail__header">
            {!singleProfile && <button
              type="button"
              className="settings-button settings-button--quiet service-back"
              disabled={pendingAction !== null}
              onClick={() => {
                dismissSettingsToast();
                setShowsEditor(false);
                setPendingConfirmation(null);
                setFeedback(null);
              }}
            >
              <Icon name="chevron-left" />
              {I18N.settings.backToServices}
            </button>}
            <div className="service-detail__identity">
              <ProviderIcon provider={selectedProfile.provider === "deepLX" ? "alibabaCloud" : selectedProfile.provider} />
              <div className="service-detail__copy">
                <div className="service-detail__title">
                  <div className="service-detail__name-help"><h2 ref={profileDetailHeading} tabIndex={-1} key={selectedProfile.name}>{profileTitle(selectedProfile)}</h2><SettingsHelp text={profileDescription(selectedProfile)} label={I18N.settings.helpLabel} /></div>
                  <div className="service-detail__status">
                    {credentialStateForTarget(selectedProfile, settings.targetLanguage) !== "present" && <CredentialBadge state={credentialStateForTarget(selectedProfile, settings.targetLanguage)} nativeSpeech={selectedProfile.provider === "appleSpeech"} />}
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
          <div className="service-detail__quick-actions">
            {singleProfile && <button id="service-add-profile" type="button"
              className="settings-button settings-button--compact settings-button--quiet"
              disabled={mutationsDisabled} onClick={openProviderPicker}>
              <Icon name="plus" />{I18N.settings.addProfile}
            </button>}
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
          </div>
          </div>
          {atProfileLimit && <InlineFeedback tone="info">{I18N.settings.profileLimitReached}</InlineFeedback>}
          <div className="service-detail__configuration">
          <ServiceEditorModule kind="setup">
          <div className="profile-form service-detail__name">
            <AutoSaveNameField key={selectedProfile.id} id={`profile-name-${selectedProfile.id}`} draftKey="profile"
              label={I18N.settings.profileName} value={selectedProfile.name} allowEmpty={false}
              placeholder={I18N.settings.profileNamePlaceholder} disabled={mutationsDisabled}
              onSave={name => handleRename(selectedProfile.id, name)} />
          </div>
          </ServiceEditorModule>
          {selectedProfile.provider === "alibabaCloud" && <ServiceEditorModule kind="models">
          <AlibabaRealtimeSettings
            key={`${selectedProfile.id}:${selectedProfile.alibabaRealtimeMode ?? "disabled"}:${selectedProfile.alibabaWorkspaceId ?? ""}`}
            profile={selectedProfile} disabled={mutationsDisabled}
            onSave={async (mode, workspaceId, savedMessage) => {
              if (mutationInFlight.current || mutationsDisabled) throw new Error("profile-change-requires-stop");
              mutationInFlight.current = true;
              setPendingAction("save-model");
              const notify = beginToast();
              try {
                await trackProfileOperation(() => updateProfile(selectedProfile.id, undefined, { alibabaRealtimeMode: mode, alibabaWorkspaceId: workspaceId }));
                notify(savedMessage);
                invalidateProfileCheck(selectedProfile.id);
              } finally { mutationInFlight.current = false; setPendingAction(null); }
            }} />
          {!usesAlibabaRealtime(selectedProfile) && textTranslationForProfile(selectedProfile) === "followService" && <QwenMTModelSettings profile={selectedProfile} disabled={mutationsDisabled}
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
}
          </ServiceEditorModule>}
          <div className="service-detail__connection service-editor-module service-editor-module--connection">
            <SelectedCredentialEditor
              support={apple.support} settings={settings} requiresStop={requiresStop} loading={apple.loading} failed={apple.failed} sourceLanguage={settings.sourceLanguage} targetLanguage={settings.targetLanguage}
              speechModeSettings={selectedProfile.provider === "xAIRealtime" ? <XAISpeechSettings profile={selectedProfile} disabled={mutationsDisabled}
                onSave={async options => {
                  if (mutationInFlight.current || mutationsDisabled) throw new Error("profile-change-requires-stop");
                  mutationInFlight.current = true;
                  setPendingAction("save-model");
                  try {
                    const snapshot = await trackProfileOperation(() => updateProfile(selectedProfile.id, undefined, options));
                    invalidateProfileCheck(selectedProfile.id);
                    return snapshot;
                  } finally { mutationInFlight.current = false; setPendingAction(null); }
                }} /> : supportsSpeechTranslationMode(selectedProfile.provider) ? <SpeechTranslationModeSettings profile={selectedProfile} disabled={mutationsDisabled}
                onSave={async options => {
                  if (mutationInFlight.current || mutationsDisabled) throw new Error("profile-change-requires-stop");
                  mutationInFlight.current = true;
                  setPendingAction("save-model");
                  try {
                    const snapshot = await trackProfileOperation(() => updateProfile(selectedProfile.id, undefined, options));
                    invalidateProfileCheck(selectedProfile.id);
                    return snapshot;
                  } finally { mutationInFlight.current = false; setPendingAction(null); }
                }} /> : undefined}
              resourceRefreshDisabled={pendingAction !== null}
              languageChangesDisabled={selectionDisabled}
              onRetry={apple.refresh} onPrepared={apple.update} onBusyChange={busy => {
                if (busy) invalidateProfileCheck(selectedProfile.id, "speech");
                setPendingAction(busy ? "prepare-resource" : null);
              }}
              onNativeTranslationBusyChange={busy => setPendingAction(busy ? "prepare-apple-translation" : null)}
              onNativeTranslationPrepared={() => invalidateProfileCheck(selectedProfile.id, "text")}
              onSelectProfile={async (profileId: string, sourceLanguage: SourceLanguage) => {
                if (selectionDisabled || mutationInFlight.current) throw new Error("profile_switch_busy");
                mutationInFlight.current = true;
                try {
                  await selectProfile(profileId, sourceLanguage);
                  invalidateProfileCheck(profileId);
                } finally { mutationInFlight.current = false; }
              }}
              connectionCheck={(draft: ProviderCredentialsInput | null | undefined, sourceLanguage?: SourceLanguage) => renderConnectionCheck(selectedProfile, usesXaiAsr(selectedProfile) || isIndependentAsrProfile(selectedProfile) || ["alibabaCloud", "deepLX"].includes(selectedProfile.provider) ? "speech" : undefined, draft, sourceLanguage)}
              textConnectionCheck={(draft) => renderConnectionCheck(selectedProfile, "text", draft)}

              key={selectedProfile.id}
              profile={selectedProfile}
              inputId={`profile-api-key-${selectedProfile.id}`}
              disabled={mutationsDisabled}
              busy={pendingAction === "save-key" || pendingAction === "delete-key"}
              visible={visible && !pickerVisible && pendingConfirmation === null}
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
            profile={selectedProfile} disabled={mutationsDisabled}
            onSave={languages => handleSaveSpeechLanguages(selectedProfile, languages)} />}
          <ServiceEditorModule kind="languages">
          <ProfileLanguagePresetSettings key={`${selectedProfile.id}-language-preset`} profile={selectedProfile} settings={settings}
            disabled={mutationsDisabled} onSave={handleSaveLanguagePreset} />
          </ServiceEditorModule>
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
          <ServiceEditorModule kind="management">
          <div className="service-detail__actions">
            <button type="button" className="settings-button settings-button--quiet settings-button--compact"
              disabled={mutationsDisabled || atProfileLimit} aria-busy={pendingAction === "duplicate" || undefined}
              onClick={() => void handleDuplicate(selectedProfile)}><Icon name="copy" />{I18N.settings.duplicateProfile}</button>
            <button
              type="button"
              className="settings-button settings-button--danger settings-button--compact"
              disabled={mutationsDisabled || settings.profiles.length <= 1}
              onClick={() => requestProfileDelete(selectedProfile)}
            >
              <Icon name="trash" />
              {I18N.settings.deleteProfile}
            </button>
          </div>
          </ServiceEditorModule>
          </div>
        </div>
      ) : !pickerVisible && (
        <div className="services-home">
          <div className="services-toolbar">
            <span className="services-toolbar__count">{I18N.settings.profileCount(settings.profiles.length)}<SettingsHelp label={I18N.settings.helpLabel} text={I18N.settings.profileSwitchHelp} /></span>
            <button
              type="button"
              className="settings-button settings-button--compact settings-button--quiet"
              disabled={mutationsDisabled || atProfileLimit}
              id="service-add-profile"
              onClick={openProviderPicker}
            >
              <Icon name="plus" />
              {I18N.settings.addProfile}
            </button>
          </div>
          {settings.profiles.length > 0 && <div className="service-rows" aria-label={I18N.settings.serviceProfilesTitle}>
            {settings.profiles.map((profile) => {
              const secondaryLabel = profileSecondaryLabel(profile);
              const isActive = profile.id === settings.activeProfileId;
              const selectionChanges = profileSelectionChangesSettings(profile, settings);
              const showsCurrentState = isActive && !selectionChanges;
              const credentialState = credentialStateForTarget(profile, profile.languagePreset?.targetLanguage ?? settings.targetLanguage);
              const credentialLabel = credentialStateText(credentialState, profile.provider === "appleSpeech");
              return (
                <div
                  className="service-row"
                  key={profile.id}
                  data-active={profile.id === settings.activeProfileId}
                >
                  <div
                    className="service-row__main"
                    role="group"
                    aria-label={`${profile.name}${isCustomSpeechProvider(profile.provider) && profile.speechRecognitionName?.trim() ? `, ${I18N.settings.speechRecognition}: ${speechRecognitionDisplayName(profile)}` : ""}${credentialState !== "present" ? `, ${credentialLabel}` : ""}${textTranslationForProfile(profile) !== "followService" ? `, ${I18N.settings.textTranslationLabel}: ${textTranslationDisplayName(profile)}` : ""}`}
                  >
                    <ProviderIcon provider={profile.provider === "deepLX" ? "alibabaCloud" : profile.provider} size={24} />
                    <span className="service-row__copy">
                      <span className="service-row__heading">
                        <strong key={profile.name} title={profile.name}>{profileTitle(profile)}</strong>
                        <ProfileTranslationRoute profile={profile} />
                        {isActive && selectionChanges && <span className="profile-active-badge">{I18N.settings.activeProfile}</span>}
                        {credentialState !== "present" && <span className="service-row__state"><CredentialBadge state={credentialState} nativeSpeech={profile.provider === "appleSpeech"} /></span>}
                      </span>
                      {profile.languagePreset && <span className="service-row__provider">{SOURCE_LANGUAGE_DISPLAY_NAMES[profile.languagePreset.sourceLanguage]} → {TARGET_LANGUAGE_DISPLAY_NAMES[profile.languagePreset.targetLanguage]}</span>}
                      {secondaryLabel && <span className="service-row__provider">{secondaryLabel}</span>}
                    </span>
                  </div>
                  <div className="service-row__actions" role="group" aria-label={profile.name}>
                    <button
                      type="button"
                      className="service-row__action service-row__use"
                      data-current={showsCurrentState}
                      disabled={selectionDisabled || !canUseProfile(profile) || !selectionChanges}
                      aria-label={`${showsCurrentState ? I18N.settings.activeProfile : I18N.settings.useProfile}: ${profile.name}`}
                      onClick={() => void handleSelect(profile.id)}
                    >
                      {showsCurrentState && <Icon name="checkmark" />}{showsCurrentState ? <span className="profile-active-badge">{I18N.settings.activeProfile}</span> : I18N.settings.profileActionUse}
                    </button>
                    <button
                      type="button"
                      id={`profile-edit-${profile.id}`}
                      className="service-row__action service-row__edit"
                      disabled={pendingAction !== null}
                      aria-label={`${I18N.settings.editProfile}: ${profile.name}`}
                      onClick={() => openEditor(profile.id)}
                    >
                      {I18N.settings.profileActionEdit}
                    </button>
                    <SettingsActionMenu id={`profile-more-${profile.id}`} label={I18N.settings.profileActionMore} name={profile.name}
                      disabled={!visible || pendingAction !== null} actions={[
                        { id: "duplicate", label: I18N.settings.profileActionCopy, icon: "copy", disabled: mutationsDisabled || atProfileLimit, onSelect: () => { void handleDuplicate(profile); } },
                        { id: "delete", label: I18N.settings.profileActionDelete, icon: "trash", destructive: true, disabled: mutationsDisabled || settings.profiles.length <= 1, onSelect: () => requestProfileDelete(profile) },
                      ]} />
                  </div>
                </div>
              );
            })}
          </div>}
          {atProfileLimit && (
            <InlineFeedback tone="info">{I18N.settings.profileLimitReached}</InlineFeedback>
          )}
        </div>
      )}
      {feedback && !pickerVisible && !(showsEditor && selectedProfile) && <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>}
    </SettingsSection>
    {pendingConfirmation?.kind === "profile" && settings.profiles.some(profile => profile.id === pendingConfirmation.profileId) && (
      <DestructiveConfirmation
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

function ProfileTranslationRoute({ profile }: { profile: ServiceProfile }) {
  if (textTranslationForProfile(profile) === "followService") return null;
  return <span className="service-row__translation" aria-label={`${I18N.settings.textTranslationLabel}: ${textTranslationDisplayName(profile)}`}>
    <span aria-hidden="true">→</span><span>{textTranslationDisplayName(profile)}</span>
  </span>;
}

function CredentialBadge({ state, nativeSpeech = false }: { state: CredentialState; nativeSpeech?: boolean }) {
  const label = credentialStateText(state, nativeSpeech);
  return <span className="credential-badge" data-state={state} aria-label={label}>{label}</span>;
}

function focusAppleSpeechResources(): boolean {
  const resources = document.getElementById("apple-speech-resources");
  if (!resources) return false;
  resources.scrollIntoView({ block: "start" });
  resources.focus({ preventScroll: true });
  return true;
}
