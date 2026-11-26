import type { ReactNode, RefObject } from "react";
import { Icon } from "../../../components/Icon";
import { ProviderIcon } from "../../../components/providers/ProviderIcon";
import { I18N } from "../../../lib/i18n";
import type { ConnectionCheckStage } from "../../../lib/ipc";
import { profileSelectionChangesSettings } from "../../../lib/profiles/profileLanguagePreset";
import { DEFAULT_NETWORK_PROXY } from "../../../lib/providers/networkProxy";
import {
  credentialStateForTarget,
  isCustomSpeechProvider,
  isIndependentAsrProfile,
  supportsSpeechTranslationMode,
  textTranslationForProfile,
  usesAlibabaRealtime,
  usesXaiAsr,
} from "../../../lib/providers/providerCapabilities";
import type {
  AlibabaRealtimeMode,
  NetworkProxyConfig,
  ProfileLanguagePreset,
  ProfileOptionsDraft,
  ProviderCredentialsInput,
  QwenMTModel,
  ServiceProfile,
  SettingsSnapshot,
  SourceLanguage,
  TextTranslationNameDraft,
} from "../../../lib/types";
import { InlineFeedback } from "../controls/SettingsPrimitives";
import { SettingsHelp } from "../feedback/SettingsHelp";
import { CustomSpeechCredentialEditor } from "./credentials/CustomSpeechCredentialEditor";
import { ProviderCredentialEditor } from "./credentials/ProviderCredentialEditor";
import { SpeechAndTextCredentialEditor } from "./credentials/SpeechAndTextCredentialEditor";
import { CustomSpeechLanguageSettings } from "./languages/CustomSpeechLanguageSettings";
import { ProfileLanguagePresetSettings } from "./languages/ProfileLanguagePresetSettings";
import { ProfileLanguageSettings } from "./languages/ProfileLanguageSettings";
import { NetworkProxySettings } from "./network/NetworkProxySettings";
import { AutoSaveNameField } from "./profiles/AutoSaveNameField";
import { profileDescription, profileTitle } from "./profiles/serviceProfilePresentation";
import { AlibabaRealtimeSettings } from "./providers/alibaba/AlibabaRealtimeSettings";
import { AppleSpeechSettings } from "./providers/apple/AppleSpeechSettings";
import type { useAppleSpeechSupport } from "./providers/apple/useAppleSpeechSupport";
import { QwenMTModelSettings } from "./providers/QwenMTModelSettings";
import { SpeechTranslationModeSettings } from "./providers/SpeechTranslationModeSettings";
import { XAISpeechSettings } from "./providers/XAISpeechSettings";
import { ServiceEditorModule } from "./ServiceEditorModule";
import { CredentialBadge } from "./ServiceProfileList";

export type ServiceProfileFeedback = { tone: "success" | "error" | "info"; message: string };

interface ServiceProfileEditorProps {
  profile: ServiceProfile;
  settings: SettingsSnapshot;
  apple: ReturnType<typeof useAppleSpeechSupport>;
  hidden: boolean;
  singleProfile: boolean;
  visible: boolean;
  requiresStop: boolean;
  actionPending: boolean;
  credentialBusy: boolean;
  duplicatePending: boolean;
  mutationsDisabled: boolean;
  selectionDisabled: boolean;
  atProfileLimit: boolean;
  canUseProfile: boolean;
  confirmingCredentialDelete: boolean;
  feedback: ServiceProfileFeedback | null;
  profileDetailHeading: RefObject<HTMLHeadingElement | null>;
  onBack: () => void;
  onOpenProviderPicker: () => void;
  onSelectProfile: (profileId: string) => void;
  onSelectAppleProfile: (profileId: string, sourceLanguage: SourceLanguage) => Promise<void>;
  onRename: (profileId: string, name: string) => Promise<SettingsSnapshot>;
  onSaveTranslationName: (profile: ServiceProfile, route: TextTranslationNameDraft["route"], name: string) => Promise<SettingsSnapshot>;
  onSaveRecognitionName: (profileId: string, name: string) => Promise<SettingsSnapshot>;
  onSaveAlibabaRealtime: (profileId: string, mode: AlibabaRealtimeMode, workspaceId: string, savedMessage: string) => Promise<void>;
  onSaveQwenMtModel: (profileId: string, model: QwenMTModel) => Promise<void>;
  onSaveSpeechOptions: (profileId: string, options: ProfileOptionsDraft) => Promise<SettingsSnapshot>;
  onSaveCredential: (profileId: string, credentials: ProviderCredentialsInput) => Promise<SettingsSnapshot | null>;
  onRequestCredentialDelete: (profileId: string) => void;
  onConfirmCredentialDelete: (profileId: string) => Promise<void>;
  onCancelCredentialDelete: () => void;
  onSaveSpeechLanguages: (profile: ServiceProfile, languages: SourceLanguage[] | null) => Promise<SettingsSnapshot>;
  onSaveLanguagePreset: (profileId: string, preset: ProfileLanguagePreset | null) => Promise<SettingsSnapshot>;
  onSaveProxy: (profile: ServiceProfile, stage: "speech" | "text", config: NetworkProxyConfig) => Promise<void>;
  onDuplicate: (profile: ServiceProfile) => void;
  onRequestDelete: (profile: ServiceProfile) => void;
  onPrepareResourceBusyChange: (profileId: string, busy: boolean) => void;
  onNativeTranslationBusyChange: (busy: boolean) => void;
  onNativeTranslationPrepared: (profileId: string) => void;
  renderConnectionCheck: (
    profile: ServiceProfile,
    stage?: ConnectionCheckStage,
    draft?: ProviderCredentialsInput | null,
    sourceLanguage?: SourceLanguage,
  ) => ReactNode;
}

export function ServiceProfileEditor({
  profile,
  settings,
  apple,
  hidden,
  singleProfile,
  visible,
  requiresStop,
  actionPending,
  credentialBusy,
  duplicatePending,
  mutationsDisabled,
  selectionDisabled,
  atProfileLimit,
  canUseProfile,
  confirmingCredentialDelete,
  feedback,
  profileDetailHeading,
  onBack,
  onOpenProviderPicker,
  onSelectProfile,
  onSelectAppleProfile,
  onRename,
  onSaveTranslationName,
  onSaveRecognitionName,
  onSaveAlibabaRealtime,
  onSaveQwenMtModel,
  onSaveSpeechOptions,
  onSaveCredential,
  onRequestCredentialDelete,
  onConfirmCredentialDelete,
  onCancelCredentialDelete,
  onSaveSpeechLanguages,
  onSaveLanguagePreset,
  onSaveProxy,
  onDuplicate,
  onRequestDelete,
  onPrepareResourceBusyChange,
  onNativeTranslationBusyChange,
  onNativeTranslationPrepared,
  renderConnectionCheck,
}: ServiceProfileEditorProps) {
  const hasIndependentTextProxy =
    !usesAlibabaRealtime(profile) &&
    (isIndependentAsrProfile(profile) || ["alibabaCloud", "deepLX"].includes(profile.provider));
  const hasRemoteTextProxy =
    hasIndependentTextProxy &&
    textTranslationForProfile(profile) !== "apple" &&
    (!isIndependentAsrProfile(profile) || textTranslationForProfile(profile) !== "followService");

  const SelectedCredentialEditor =
    profile.provider === "appleSpeech"
      ? AppleSpeechSettings
      : isCustomSpeechProvider(profile.provider)
        ? CustomSpeechCredentialEditor
        : ["alibabaCloud", "deepLX"].includes(profile.provider)
          ? SpeechAndTextCredentialEditor
          : ProviderCredentialEditor;

  const speechModeSettings =
    profile.provider === "xAIRealtime" ? (
      <XAISpeechSettings
        profile={profile}
        disabled={mutationsDisabled}
        onSave={(options) => onSaveSpeechOptions(profile.id, options)}
      />
    ) : supportsSpeechTranslationMode(profile.provider) ? (
      <SpeechTranslationModeSettings
        profile={profile}
        disabled={mutationsDisabled}
        onSave={(options) => onSaveSpeechOptions(profile.id, options)}
      />
    ) : undefined;

  return (
    <div className="service-detail" hidden={hidden}>
      <div className="service-detail__header">
        {!singleProfile && (
          <button
            type="button"
            className="settings-button settings-button--quiet service-back"
            disabled={actionPending}
            onClick={onBack}
          >
            <Icon name="chevron-left" />
            {I18N.settings.backToServices}
          </button>
        )}
        <div className="service-detail__identity">
          <ProviderIcon provider={profile.provider === "deepLX" ? "alibabaCloud" : profile.provider} />
          <div className="service-detail__copy">
            <div className="service-detail__title">
              <div className="service-detail__name-help">
                <h2 ref={profileDetailHeading} tabIndex={-1} key={profile.name}>{profileTitle(profile)}</h2>
                <SettingsHelp text={profileDescription(profile)} label={I18N.settings.helpLabel} />
              </div>
              <div className="service-detail__status">
                {credentialStateForTarget(profile, settings.targetLanguage) !== "present" && (
                  <CredentialBadge
                    state={credentialStateForTarget(profile, settings.targetLanguage)}
                    nativeSpeech={profile.provider === "appleSpeech"}
                  />
                )}
                {profile.id === settings.activeProfileId && (
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
          {singleProfile && (
            <button
              id="service-add-profile"
              type="button"
              className="settings-button settings-button--compact settings-button--quiet"
              disabled={mutationsDisabled}
              onClick={onOpenProviderPicker}
            >
              <Icon name="plus" />
              {I18N.settings.addProfile}
            </button>
          )}
          {canUseProfile && profileSelectionChangesSettings(profile, settings) && (
            <span className="service-detail__use">
              <button
                type="button"
                className="settings-button settings-button--quiet settings-button--compact"
                disabled={selectionDisabled}
                onClick={() => onSelectProfile(profile.id)}
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
            <AutoSaveNameField
              key={profile.id}
              id={`profile-name-${profile.id}`}
              draftKey="profile"
              label={I18N.settings.profileName}
              value={profile.name}
              allowEmpty={false}
              placeholder={I18N.settings.profileNamePlaceholder}
              disabled={mutationsDisabled}
              onSave={(name) => onRename(profile.id, name)}
            />
          </div>
        </ServiceEditorModule>
        {profile.provider === "alibabaCloud" && (
          <ServiceEditorModule kind="models">
            <AlibabaRealtimeSettings
              key={`${profile.id}:${profile.alibabaRealtimeMode ?? "disabled"}:${profile.alibabaWorkspaceId ?? ""}`}
              profile={profile}
              disabled={mutationsDisabled}
              onSave={(mode, workspaceId, savedMessage) =>
                onSaveAlibabaRealtime(profile.id, mode, workspaceId, savedMessage)}
            />
            {!usesAlibabaRealtime(profile) && textTranslationForProfile(profile) === "followService" && (
              <QwenMTModelSettings
                profile={profile}
                disabled={mutationsDisabled}
                onSave={(model) => onSaveQwenMtModel(profile.id, model)}
              />
            )}
          </ServiceEditorModule>
        )}
        <div className="service-detail__connection service-editor-module service-editor-module--connection">
          <SelectedCredentialEditor
            support={apple.support}
            settings={settings}
            requiresStop={requiresStop}
            loading={apple.loading}
            failed={apple.failed}
            sourceLanguage={settings.sourceLanguage}
            targetLanguage={settings.targetLanguage}
            translationLanguageControls={profile.id === settings.activeProfileId ? () => (
              // Language writes belong to the active configuration, not the
              // inactive profile whose credentials happen to be open.
              <ProfileLanguageSettings settings={settings} embedded hideSourceLanguage busy={selectionDisabled} />
            ) : undefined}
            speechModeSettings={speechModeSettings}
            resourceRefreshDisabled={actionPending}
            languageChangesDisabled={selectionDisabled}
            onRetry={apple.refresh}
            onPrepared={apple.update}
            onBusyChange={(busy) => onPrepareResourceBusyChange(profile.id, busy)}
            onNativeTranslationBusyChange={onNativeTranslationBusyChange}
            onNativeTranslationPrepared={() => onNativeTranslationPrepared(profile.id)}
            onSelectProfile={onSelectAppleProfile}
            connectionCheck={(draft: ProviderCredentialsInput | null | undefined, sourceLanguage?: SourceLanguage) =>
              renderConnectionCheck(
                profile,
                usesXaiAsr(profile) || isIndependentAsrProfile(profile) || ["alibabaCloud", "deepLX"].includes(profile.provider)
                  ? "speech"
                  : undefined,
                draft,
                sourceLanguage,
              )}
            textConnectionCheck={(draft) => renderConnectionCheck(profile, "text", draft)}
            key={profile.id}
            profile={profile}
            inputId={`profile-api-key-${profile.id}`}
            disabled={mutationsDisabled}
            busy={credentialBusy}
            visible={visible}
            feedback={feedback}
            onSaveTranslationName={(route, name) => onSaveTranslationName(profile, route, name)}
            onSaveRecognitionName={(name: string) => onSaveRecognitionName(profile.id, name)}
            onSave={(replacement) => onSaveCredential(profile.id, replacement)}
            onRequestDelete={() => onRequestCredentialDelete(profile.id)}
            onConfirmDelete={() => onConfirmCredentialDelete(profile.id)}
            confirmingDelete={confirmingCredentialDelete}
            onCancelDelete={onCancelCredentialDelete}
          />
        </div>
        {isCustomSpeechProvider(profile.provider) && (
          <CustomSpeechLanguageSettings
            key={`${profile.id}-languages`}
            profile={profile}
            disabled={mutationsDisabled}
            onSave={(languages) => onSaveSpeechLanguages(profile, languages)}
          />
        )}
        <ServiceEditorModule kind="languages">
          <ProfileLanguagePresetSettings
            key={`${profile.id}-language-preset`}
            profile={profile}
            settings={settings}
            disabled={mutationsDisabled}
            onSave={(preset) => onSaveLanguagePreset(profile.id, preset)}
          />
        </ServiceEditorModule>
        {(profile.provider !== "appleSpeech" || hasRemoteTextProxy) && (
          <section className="service-proxies" aria-label={I18N.settings.networkProxyTitle}>
            <h3>{I18N.settings.networkProxyTitle}</h3>
            {profile.provider !== "appleSpeech" && (
              <NetworkProxySettings
                key={`${profile.id}-speech-proxy`}
                embedded
                label={hasIndependentTextProxy ? I18N.settings.speechRecognition : I18N.settings.voiceTranslation}
                scope={hasIndependentTextProxy ? I18N.settings.networkProxySpeechScope : I18N.settings.networkProxyIntegratedScope}
                value={profile.speechNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY}
                disabled={mutationsDisabled}
                onSave={(config) => onSaveProxy(profile, "speech", config)}
              />
            )}
            {hasRemoteTextProxy && (
              <NetworkProxySettings
                key={`${profile.id}-text-proxy`}
                embedded
                label={I18N.settings.textTranslationLabel}
                scope={I18N.settings.networkProxyTextScope}
                value={profile.textNetworkProxy ?? settings.networkProxy ?? DEFAULT_NETWORK_PROXY}
                disabled={mutationsDisabled}
                onSave={(config) => onSaveProxy(profile, "text", config)}
              />
            )}
          </section>
        )}
        <ServiceEditorModule kind="management">
          <div className="service-detail__actions">
            <button
              type="button"
              className="settings-button settings-button--quiet settings-button--compact"
              disabled={mutationsDisabled || atProfileLimit}
              aria-busy={duplicatePending || undefined}
              onClick={() => onDuplicate(profile)}
            >
              <Icon name="copy" />
              {I18N.settings.duplicateProfile}
            </button>
            <button
              type="button"
              className="settings-button settings-button--danger settings-button--compact"
              disabled={mutationsDisabled || settings.profiles.length <= 1}
              onClick={() => onRequestDelete(profile)}
            >
              <Icon name="trash" />
              {I18N.settings.deleteProfile}
            </button>
          </div>
        </ServiceEditorModule>
      </div>
    </div>
  );
}
