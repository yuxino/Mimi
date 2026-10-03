import { testProfileConnection, type ConnectionCheckStage, type ConnectionDiagnostic, type StoredCredentialField } from "../../lib/ipc";
import { profileErrorMessage, diagnosticCopy } from "../../lib/connectionDiagnostics";
import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Tooltip } from "../../components/Tooltip";
import { Icon } from "../../components/Icon";
import { ProviderIcon } from "../../components/ProviderIcon";
import { I18N, providerDisplayName } from "../../lib/i18n";
import { SERVICE_PROVIDERS, credentialStateForTarget, isCustomSpeechProvider, subtitlePreferencesChanged, textTranslationForProfile } from "../../lib/providerCapabilities";
import { DEFAULT_NETWORK_PROXY, networkProxyConfigKey } from "../../lib/networkProxy";
import {
  buildProviderCredentials,
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
  ServiceProvider,
  SessionStateEvent,
  SettingsSnapshot,
} from "../../lib/types";
import { InlineFeedback, SettingsSection } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

import { DestructiveConfirmation, SettingsConfirmation } from "./DestructiveConfirmation";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";
import { CustomSpeechCredentialEditor } from "./CustomSpeechCredentialEditor";

import { ConfigInput } from "./ConfigInput";
import { SettingsHelp } from "./SettingsHelp";
import { ConnectionCheck } from "./ConnectionCheck";
import { saveAndSelectProfile } from "./saveAndSelectProfile";
import { StoredCredentialReveal } from "./StoredCredentialReveal";
import { SettingsInitializationStatus } from "./SettingsInitializationStatus";
import { NetworkProxySettings } from "./NetworkProxySettings";
import { ProfileLanguageSettings } from "./ProfileLanguageSettings";

const CONNECTION_CHECK_TIMEOUT_MS = 30_000;

type Feedback = { tone: "success" | "error" | "info"; message: string };
type PendingAction = "create" | "rename" | "select" | "delete" | "save-key" | "delete-key" | "test-connection" | "save-proxy" | null;
type CheckStage = ConnectionCheckStage | "combined";
type CheckOutcome = { profileId: string; result: ConnectionDiagnostic | null; error: string | null };
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
}: {
  settings: SettingsSnapshot;
  sessionIsActive: boolean;
  sessionIsPaused?: boolean;
  sessionStatusKind?: SessionStateEvent["status"]["kind"];
  visible?: boolean;
  overview?: ReactNode;
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

  const activeProfile =
    settings.profiles.find((profile) => profile.id === settings.activeProfileId) ??
    settings.profiles[0];
  const [selectedProfileId, setSelectedProfileId] = useState(
    activeProfile?.id ?? settings.activeProfileId,
  );
  const [showsEditor, setShowsEditor] = useState(false);
  const [showsProviderPicker, setShowsProviderPicker] = useState(false);
  const [nameDraft, setNameDraft] = useState(activeProfile?.name ?? "");
  const [pendingAction, setPendingAction] = useState<PendingAction>(null);
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
  const [renderedProfile, setRenderedProfile] = useState({
    id: activeProfile?.id,
    name: activeProfile?.name,
  });

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
    (selectedProfile.id !== renderedProfile.id || selectedProfile.name !== renderedProfile.name)
  ) {
    setRenderedProfile({
      id: selectedProfile.id,
      name: selectedProfile.name,
    });
    setNameDraft(selectedProfile.name);
    setFeedback(null);
    setPendingConfirmation(null);
  }

  const hasIndependentTextProxy = !!selectedProfile && (isCustomSpeechProvider(selectedProfile.provider) || ["alibabaCloud", "deepLX"].includes(selectedProfile.provider));

  const SelectedCredentialEditor = selectedProfile && isCustomSpeechProvider(selectedProfile.provider) ? CustomSpeechCredentialEditor : selectedProfile && ["alibabaCloud", "deepLX"].includes(selectedProfile.provider) ? AlibabaCredentialEditor : CredentialEditor;

  const requiresStop = sessionIsActive || sessionIsPaused || sessionStatusKind === "connecting" || sessionStatusKind === "stopping";
  const mutationsDisabled = requiresStop || pendingAction !== null;
  const atProfileLimit = settings.profiles.filter(profile => profile.credentialStorage !== "localDevFile").length >= 20;

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
      const snapshot = await operation();
      const result: Feedback = typeof successFeedback === "string"
          ? { tone: "success", message: successFeedback }
          : successFeedback(snapshot);
      if (result.tone !== "error") notify(result.message);
      else setFeedback(result);
      return snapshot;
    } catch (error) {
      setFeedback({
        tone: "error",
        message: profileErrorMessage(error),
      });
      return null;
    } finally {
      mutationInFlight.current = false;
      setPendingAction(null);
    }
  };

  const handleCreate = async (provider: ServiceProvider) => {
    if (creationInFlight.current || mutationsDisabled || atProfileLimit) return;
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

  const handleRename = async () => {
    if (!selectedProfile) return;
    const name = nameDraft.trim();
    if (!name || name === selectedProfile.name) return;
    await perform(
      "rename",
      () => updateProfile(selectedProfile.id, name),
      I18N.settings.profileNameSaved,
    );
  };

  const handleSaveProxy = async (profile: ServiceProfile, stage: "speech" | "text", config: NetworkProxyConfig) => {
    if (mutationInFlight.current || mutationsDisabled) throw new Error("network_proxy_change_requires_stop");
    mutationInFlight.current = true;
    setPendingAction("save-proxy");
    try {
      await updateProfile(profile.id, profile.name, stage === "speech" ? { speechNetworkProxy: config } : { textNetworkProxy: config });
      invalidateProfileCheck(profile.id, stage);
    } finally {
      mutationInFlight.current = false;
      setPendingAction(null);
    }
  };

  const handleSelect = async (profileId: string) => {
    if (profileId === settings.activeProfileId) return;
    setPendingConfirmation(null);
    setSelectedProfileId(profileId);
    await perform(
      "select",
      () => selectProfile(profileId),
      (snapshot) =>
        subtitlePreferencesChanged(settings, snapshot)
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
    const stage = credentials.kind === "customSpeech" ? "speech" : credentials.kind === "alibabaTranslation" && (isCustomSpeechProvider(selectedProfile!.provider) || !credentials.apiKey.trim()) ? "text" : undefined;
    invalidateProfileCheck(profileId, stage);
    return perform(
      "save-key",
      () => saveAndSelectProfile(profileId, credentials, saveProfileCredentials, selectProfile),
      (snapshot) => ({
        tone: subtitlePreferencesChanged(settings, snapshot) ? "info" : "success",
        message: subtitlePreferencesChanged(settings, snapshot)
          ? I18N.settings.profileSelectedWithAdjustments
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

  const handleConnectionCheck = (stage?: ConnectionCheckStage) => {
    if (!selectedProfile || pendingAction !== null || checkInFlight.current) return;
    checkInFlight.current = true;
    const profileId = selectedProfile.id;
    const key = stage ?? "combined";
    const publish = (result: ConnectionDiagnostic | null, error: string | null) => setDiagnostics(current => ({ ...current, [key]: { profileId, result, error } }));
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
    void (stage ? testProfileConnection(profileId, stage) : testProfileConnection(profileId))
      .then((result) => {
        if (canPublish()) publish(result, null);
      })
      .catch(() => {
        if (canPublish()) publish(null, `${diagnosticCopy().unavailable}: ${diagnosticCopy().checkFailed}`);
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

  const renderConnectionCheck = (profile: ServiceProfile, stage?: ConnectionCheckStage, requiresSave = false) => {
    const key = stage ?? "combined";
    const outcome = diagnostics[key];
    return <ConnectionCheck result={!requiresSave && outcome?.profileId === profile.id ? outcome.result : null} error={!requiresSave && outcome?.profileId === profile.id ? outcome.error : null} pending={pendingCheckStage === key} disabled={mutationsDisabled || requiresSave} requiresSave={requiresSave} onCheck={() => handleConnectionCheck(stage)} label={stage === "text" ? I18N.settings.checkTextTranslation : stage === "speech" ? I18N.settings.checkSpeechRecognition : undefined} />;
  };

  if (initializationStatus !== "ready") {
    return <SettingsInitializationStatus status={initializationStatus} error={initializationError} onRetry={() => { void initialize(); }} />;
  }

  return (
    <>
    {!showsEditor && !showsProviderPicker && overview}
    <SettingsSection id="service-profiles" title={I18N.settings.serviceProfilesTitle} hideHeading>
      {(sessionIsActive || sessionIsPaused) && (
        <InlineFeedback tone="info" icon="lock">
          {I18N.settings.profileMutationsLocked}
        </InlineFeedback>
      )}
      {showsProviderPicker ? (
        <ProviderPicker
          disabled={mutationsDisabled}
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
                  <h2>{profileTitle(selectedProfile)}</h2>
                  <div className="service-detail__status">
                    <CredentialBadge state={credentialStateForTarget(selectedProfile, settings.targetLanguage)} />
                    {selectedProfile.id === settings.activeProfileId && (
                      <span className="profile-active-badge">
                        <Icon name="checkmark" />
                        {I18N.settings.activeProfile}
                      </span>
                    )}
                  </div>
                </div>
                <span className="service-detail__description"><SettingsHelp text={profileDescription(selectedProfile)} label={I18N.settings.helpLabel} /></span>
              </div>
            </div>
          </div>
          <div className="service-detail__configuration">
          <form
            className="profile-form service-detail__name"
            onSubmit={(event) => { event.preventDefault(); void handleRename(); }}
          >
            <div className="settings-field">
              <label htmlFor={`profile-name-${selectedProfile.id}`}>{I18N.settings.profileName}</label>
              <span className="settings-field__inline">
                <input id={`profile-name-${selectedProfile.id}`} value={nameDraft} maxLength={64} disabled={mutationsDisabled} readOnly={selectedProfileReadOnly} placeholder={I18N.settings.profileNamePlaceholder} onChange={(event) => { setNameDraft(event.target.value); setFeedback(null); }} />
                {nameDraft.trim() !== selectedProfile.name && <button type="submit" className="settings-link service-detail__save-name" disabled={mutationsDisabled || !nameDraft.trim()}>{I18N.settings.saveName}</button>}
              </span>
            </div>
          </form>
          <div className="service-detail__connection">
            <SelectedCredentialEditor
              connectionCheck={renderConnectionCheck(selectedProfile, isCustomSpeechProvider(selectedProfile.provider) || ["alibabaCloud", "deepLX"].includes(selectedProfile.provider) ? "speech" : undefined)}
              textConnectionCheck={(requiresSave) => renderConnectionCheck(selectedProfile, "text", requiresSave)}
              readOnly={selectedProfileReadOnly}
              key={selectedProfile.id}
              profile={selectedProfile}
              inputId={`profile-api-key-${selectedProfile.id}`}
              disabled={mutationsDisabled}
              busy={pendingAction === "save-key" || pendingAction === "delete-key"}
              visible={visible && pendingConfirmation === null}
              feedback={feedback}
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
          <section className="service-proxies" aria-label={I18N.settings.networkProxyTitle}>
            <h3>{I18N.settings.networkProxyTitle}</h3>
            <NetworkProxySettings key={`${selectedProfile.id}-speech-proxy`} embedded
              label={hasIndependentTextProxy ? I18N.settings.speechRecognition : I18N.settings.voiceTranslation}
              scope={hasIndependentTextProxy ? I18N.settings.networkProxySpeechScope : I18N.settings.networkProxyIntegratedScope}
              value={selectedProfile.speechNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY} disabled={mutationsDisabled}
              onSave={config => handleSaveProxy(selectedProfile, "speech", config)} />
            {hasIndependentTextProxy && <NetworkProxySettings key={`${selectedProfile.id}-text-proxy`} embedded
              label={I18N.settings.textTranslationLabel} scope={I18N.settings.networkProxyTextScope}
              value={selectedProfile.textNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY} disabled={mutationsDisabled}
              onSave={config => handleSaveProxy(selectedProfile, "text", config)} />}
          </section>
          {selectedProfile.id === settings.activeProfileId
            ? <ProfileLanguageSettings key={selectedProfile.id} settings={settings} disabled={mutationsDisabled} requiresStop={requiresStop} />
            : null}
          <div className="service-detail__actions">
            {selectedProfile.id !== settings.activeProfileId && <SettingsHelp text={I18N.settings.useProfileForLanguages} label={I18N.settings.helpLabel} />}
            {credentialStateForTarget(selectedProfile, settings.targetLanguage) === "present" &&
              selectedProfile.id !== settings.activeProfileId && (
                <button
                  type="button"
                  className="settings-button settings-button--quiet settings-button--compact"
                  disabled={mutationsDisabled}
                  onClick={() => void handleSelect(selectedProfile.id)}
                >
                  <Icon name="checkmark" />
                  {I18N.settings.useProfile}
                </button>
              )}
            {selectedProfileReadOnly ? <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={mutationsDisabled || atProfileLimit} onClick={() => setShowsProviderPicker(true)}><Icon name="plus" />{I18N.settings.addProfile}</button> : <button
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
            <span>{I18N.settings.profileCount(settings.profiles.length)}</span>
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
                  disabled={mutationsDisabled}
                  onClick={() => {
                    if (
                      credentialStateForTarget(profile, settings.targetLanguage) === "present" &&
                      profile.id !== settings.activeProfileId
                    )
                      void handleSelect(profile.id);
                    else openEditor(profile.id);
                  }}
                  aria-label={`${profile.name}, ${credentialStateText(credentialStateForTarget(profile, settings.targetLanguage))}${textTranslationForProfile(profile) !== "followService" ? `, ${I18N.settings.textTranslationLabel}: ${translationName(profile)}` : ""}: ${credentialStateForTarget(profile, settings.targetLanguage) === "present" && profile.id !== settings.activeProfileId ? I18N.settings.useProfile : I18N.settings.editProfile}`}
                >
                  <ProviderIcon provider={profile.provider === "deepLX" ? "alibabaCloud" : profile.provider} />
                  <span className="service-row__copy">
                    <strong>{profileTitle(profile)}</strong>
                    {profileSecondaryLabel(profile) && <span className="service-row__provider">{profileSecondaryLabel(profile)}</span>}
                    {textTranslationForProfile(profile) !== "followService" && <span className="service-row__translation"><ProviderIcon provider={textTranslationForProfile(profile) as "deepL" | "deepLX" | "openAICompatible" | "chatMock"} size={32} /><span>{I18N.settings.textTranslationLabel} · {translationName(profile)}</span></span>}
                  </span>
                  <span className="service-row__state">
                    <CredentialBadge state={credentialStateForTarget(profile, settings.targetLanguage)} />
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
                  disabled={mutationsDisabled}
                  aria-label={`${I18N.settings.editProfile}: ${profile.name}`}
                  onClick={() => openEditor(profile.id)}
                >
                  <Icon name="chevron-right" />
                </button>
              </div>
            ))}
          </div>
          <div className="services-hint"><SettingsHelp icon="shield-check" label={I18N.settings.helpLabel} text={settings.profiles.some(profile => profile.credentialStorage === "localDevFile") ? diagnosticCopy().localDevReadOnly : I18N.settings.servicesHint} /></div>
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
  connectionCheck?: ReactNode;
  textConnectionCheck?: (requiresSave: boolean) => ReactNode;
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
  const [endpointInvalid, setEndpointInvalid] = useState(false);
  const endpointRef = useRef<HTMLInputElement>(null);
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
  const credentials = buildProviderCredentials(profile.provider, draft);

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!credentials) return;
    if (profile.provider === "deepLX" && !deepLXEndpointIsValid(draft.endpoint)) {
      setEndpointInvalid(true);
      endpointRef.current?.focus();
      return;
    }
    setEndpointInvalid(false);
    setEditingSavedCredential(true);
    // Saved-value previews are independent of this replacement draft. Keep
    // only the user's edits on failure; discard after a successful save/use.
    void onSave(credentials).then((saved) => {
      if (saved) {
        setDraft(emptyCredentialDraft());
        setEditingSavedCredential(false);
      }
    });
  };

  const handleConfirmDelete = () => {
    const next = credentialEditorStateAfterDeleteRequest({ draft, editingSavedCredential }, true);
    // Clear plaintext before the async keychain deletion starts. A failure
    // must never restore a replacement secret to WebView state or the DOM.
    setDraft(next.draft);
    setEditingSavedCredential(next.editingSavedCredential);
    void onConfirmDelete();
  };

  const serviceIdentity = <section className="service-stage service-stage--integrated">
    <header className="service-stage__heading"><h3>{I18N.settings.voiceTranslation}</h3><SettingsHelp text={I18N.settings.textTranslationUnsupported} label={I18N.settings.helpLabel} /></header>
    <div className="settings-field service-stage__selector"><span>{I18N.settings.serviceProvider}</span><span className="service-stage__provider"><ProviderIcon provider={profile.provider} size={32} />{providerDisplayName(profile.provider)}</span></div>
  </section>;
  const storageHelp = <SettingsHelp id={noteId} text={readOnly ? profile.credentialState === "unavailable" ? diagnosticCopy().localDevUnavailable : diagnosticCopy().localDevReadOnly : I18N.settings.credentialNote} icon="shield-check" label={I18N.settings.helpLabel} />;
  if (readOnly) return <div className="credential-panel"><div className="service-credential-toolbar">{connectionCheck}{storageHelp}</div>{serviceIdentity}</div>;

  if (profile.credentialState === "present" && !editingSavedCredential) {
    return (
      <div className="credential-panel credential-panel--saved">
        <div className="service-credential-toolbar">{connectionCheck}<span className="credential-panel__saved-actions">
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
        </span>{storageHelp}</div>
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
      <div className="service-credential-toolbar">{connectionCheck}{storageHelp}</div>
      {serviceIdentity}
      <div className="credential-panel__heading">
        <span>
          <span className="credential-panel__label">{I18N.settings.credentials}</span>
        </span>
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
        <div className="credential-form__fields">
          {credentialFieldsForProvider(profile.provider).map((field) => {
            const copy = credentialFieldCopy(field, profile.provider);
            const fieldId = `${inputId}-${field}`;
            return (
              <div className="settings-field" key={field}>
                <label htmlFor={fieldId}>{copy.label}</label>
                <ConfigInput
                  id={fieldId}
                  ref={profile.provider === "deepLX" && field === "endpoint" ? endpointRef : undefined}
                  aria-invalid={field === "endpoint" && endpointInvalid ? true : undefined}
                  type={copy.secret ? "password" : "text"}
                  inputMode={field === "appId" ? "numeric" : undefined}
                  value={draft[field]}
                  autoComplete="new-password"
                  spellCheck={false}
                  aria-describedby={field === "endpoint" && endpointInvalid ? `${fieldId}-error ${noteId}` : noteId}
                  disabled={disabled}
                  placeholder={copy.placeholder}
                  onValueChange={(value) => {
                    setEditingSavedCredential(true);
                    setDraft((current) => ({ ...current, [field]: value }));
                    if (field === "endpoint" && endpointInvalid) setEndpointInvalid(!deepLXEndpointIsValid(value));
                  }}
                />
                {copy.secret && profile.credentialState === "present" && visible && !busy && !confirmingDelete && <StoredCredentialReveal key={`${profile.id}:${field}`} profileId={profile.id} field={field as StoredCredentialField} label={copy.label} disabled={disabled} />}
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
  if (isCustomSpeechProvider(profile.provider)) return [I18N.settings.customSpeechDescription, I18N.settings.customSpeechLanguages].join("\n");
  const translation = textTranslationForProfile(profile);
  return translation === "deepL" ? I18N.settings.deepLChain : translation === "deepLX" ? I18N.settings.deepLXChain : (translation === "openAICompatible" || translation === "chatMock") ? I18N.settings.openAICompatibleChain : providerDescription(profile.provider);
}

function translationName(profile: ServiceProfile): string {
  const translation = textTranslationForProfile(profile);
  return translation === "deepL" ? "DeepL" : translation === "deepLX" ? "DeepLX" : translation === "chatMock" ? "ChatMock" : translation === "openAICompatible" ? I18N.settings.textTranslationOpenAICompatible : profileProviderName(profile);
}

function profileSecondaryLabel(profile: ServiceProfile): string | null {
  if (textTranslationForProfile(profile) !== "followService") return `${I18N.settings.speechRecognition} · ${profileProviderName(profile)}`;
  const provider = profileProviderName(profile);
  return profileTitle(profile).trim().toLowerCase() === provider.toLowerCase() ? null : provider;
}

function credentialFieldCopy(field: CredentialFieldName, provider: ServiceProvider): {
  label: string;
  placeholder: string;
  secret: boolean;
} {
  switch (field) {
    case "model":
      return { label: I18N.settings.customSpeechModel, placeholder: "recognition-model", secret: false };
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
        placeholder: provider === "deepLX" ? "https://example.com/translate" : I18N.settings.azureEndpointPlaceholder,
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
  disabled,
  feedback,
  onChoose,
  onCancel,
}: {
  disabled: boolean;
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
        <button type="button" className="settings-link" disabled={disabled} onClick={onCancel}>{I18N.settings.cancel}</button>
      </div>
      {!selectedProvider && feedback && <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>}
      <div className="provider-picker__options">
        {SERVICE_PROVIDERS.map((provider) => (
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
      {selectedProvider && <SettingsConfirmation message={I18N.settings.confirmAddProfile} disabled={disabled} variant="default" confirmLabel={I18N.settings.confirmAddProfile} confirmIcon="plus" onCancel={() => { if (!disabled) setSelectedProvider(null); }} onConfirm={() => onChoose(selectedProvider)}>
        <div className="provider-picker__preview">
          <ProviderIcon provider={selectedProvider} />
          <h3>{providerDisplayName(selectedProvider)}</h3>
          <SettingsHelp text={providerDescription(selectedProvider)} label={I18N.settings.helpLabel} />
        </div>
        {feedback && <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>}
      </SettingsConfirmation>}
    </div>
  );
}

function CredentialBadge({ state }: { state: CredentialState }) {
  const label = credentialStateText(state);
  return <Tooltip label={label} popupClassName="settings-help-tooltip">{(descriptionId) => <span className="credential-badge" data-state={state} role="img" aria-label={label} aria-describedby={descriptionId} tabIndex={0}><Icon name={state === "present" ? "shield-check" : state === "missing" ? "key" : "exclamation-triangle"} /></span>}</Tooltip>;
}

function providerDescription(provider: ServiceProvider): string {
  switch (provider) {
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

function credentialStateText(state: CredentialState): string {
  switch (state) {
    case "present":
      return I18N.settings.credentialPresent;
    case "missing":
      return I18N.settings.credentialMissing;
    case "unavailable":
      return I18N.settings.credentialUnavailable;
  }
}
