import { Icon } from "../../../components/Icon";
import { ProviderIcon } from "../../../components/providers/ProviderIcon";
import { I18N } from "../../../lib/i18n";
import { profileSelectionChangesSettings } from "../../../lib/profiles/profileLanguagePreset";
import {
  credentialStateForTarget,
  isCustomSpeechProvider,
  textTranslationForProfile,
} from "../../../lib/providers/providerCapabilities";
import { speechRecognitionDisplayName } from "../../../lib/providers/speechRecognitionName";
import { textTranslationDisplayName } from "../../../lib/providers/textTranslationName";
import {
  SOURCE_LANGUAGE_DISPLAY_NAMES,
  TARGET_LANGUAGE_DISPLAY_NAMES,
  type CredentialState,
  type ServiceProfile,
  type SettingsSnapshot,
} from "../../../lib/types";
import { SettingsActionMenu } from "../controls/SettingsActionMenu";
import { InlineFeedback } from "../controls/SettingsPrimitives";
import { SettingsHelp } from "../feedback/SettingsHelp";
import {
  credentialStateText,
  profileSecondaryLabel,
  profileTitle,
} from "./profiles/serviceProfilePresentation";

interface ServiceProfileListProps {
  settings: SettingsSnapshot;
  visible: boolean;
  actionPending: boolean;
  mutationsDisabled: boolean;
  selectionDisabled: boolean;
  atProfileLimit: boolean;
  canUseProfile: (profile: ServiceProfile) => boolean;
  onOpenProviderPicker: () => void;
  onSelectProfile: (profileId: string) => void;
  onOpenEditor: (profileId: string) => void;
  onDuplicateProfile: (profile: ServiceProfile) => void;
  onRequestDeleteProfile: (profile: ServiceProfile) => void;
}

export function ServiceProfileList({
  settings,
  visible,
  actionPending,
  mutationsDisabled,
  selectionDisabled,
  atProfileLimit,
  canUseProfile,
  onOpenProviderPicker,
  onSelectProfile,
  onOpenEditor,
  onDuplicateProfile,
  onRequestDeleteProfile,
}: ServiceProfileListProps) {
  return (
    <div className="services-home">
      <div className="services-toolbar">
        <span className="services-toolbar__count">
          {I18N.settings.profileCount(settings.profiles.length)}
          <SettingsHelp label={I18N.settings.helpLabel} text={I18N.settings.profileSwitchHelp} />
        </span>
        <button
          type="button"
          className="settings-button settings-button--compact settings-button--quiet"
          disabled={mutationsDisabled || atProfileLimit}
          id="service-add-profile"
          onClick={onOpenProviderPicker}
        >
          <Icon name="plus" />
          {I18N.settings.addProfile}
        </button>
      </div>
      {settings.profiles.length > 0 && (
        <div className="service-rows" aria-label={I18N.settings.serviceProfilesTitle}>
          {settings.profiles.map((profile) => {
            const secondaryLabel = profileSecondaryLabel(profile);
            const isActive = profile.id === settings.activeProfileId;
            const selectionChanges = profileSelectionChangesSettings(profile, settings);
            const showsCurrentState = isActive && !selectionChanges;
            const credentialState = credentialStateForTarget(
              profile,
              profile.languagePreset?.targetLanguage ?? settings.targetLanguage,
            );
            const credentialLabel = credentialStateText(credentialState, profile.provider === "appleSpeech");
            return (
              <div
                className="service-row"
                key={profile.id}
                data-active={isActive}
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
                      {isActive && selectionChanges && (
                        <span className="profile-active-badge">{I18N.settings.activeProfile}</span>
                      )}
                      {credentialState !== "present" && (
                        <span className="service-row__state">
                          <CredentialBadge state={credentialState} nativeSpeech={profile.provider === "appleSpeech"} />
                        </span>
                      )}
                    </span>
                    {profile.languagePreset && (
                      <span className="service-row__provider">
                        {SOURCE_LANGUAGE_DISPLAY_NAMES[profile.languagePreset.sourceLanguage]} → {TARGET_LANGUAGE_DISPLAY_NAMES[profile.languagePreset.targetLanguage]}
                      </span>
                    )}
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
                    onClick={() => onSelectProfile(profile.id)}
                  >
                    {showsCurrentState && <Icon name="checkmark" />}
                    {showsCurrentState
                      ? <span className="profile-active-badge">{I18N.settings.activeProfile}</span>
                      : I18N.settings.profileActionUse}
                  </button>
                  <button
                    type="button"
                    id={`profile-edit-${profile.id}`}
                    className="service-row__action service-row__edit"
                    disabled={actionPending}
                    aria-label={`${I18N.settings.editProfile}: ${profile.name}`}
                    onClick={() => onOpenEditor(profile.id)}
                  >
                    {I18N.settings.profileActionEdit}
                  </button>
                  <SettingsActionMenu
                    id={`profile-more-${profile.id}`}
                    label={I18N.settings.profileActionMore}
                    name={profile.name}
                    disabled={!visible || actionPending}
                    actions={[
                      {
                        id: "duplicate",
                        label: I18N.settings.profileActionCopy,
                        icon: "copy",
                        disabled: mutationsDisabled || atProfileLimit,
                        onSelect: () => { onDuplicateProfile(profile); },
                      },
                      {
                        id: "delete",
                        label: I18N.settings.profileActionDelete,
                        icon: "trash",
                        destructive: true,
                        disabled: mutationsDisabled || settings.profiles.length <= 1,
                        onSelect: () => onRequestDeleteProfile(profile),
                      },
                    ]}
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}
      {atProfileLimit && (
        <InlineFeedback tone="info">{I18N.settings.profileLimitReached}</InlineFeedback>
      )}
    </div>
  );
}

function ProfileTranslationRoute({ profile }: { profile: ServiceProfile }) {
  if (textTranslationForProfile(profile) === "followService") return null;
  return (
    <span
      className="service-row__translation"
      aria-label={`${I18N.settings.textTranslationLabel}: ${textTranslationDisplayName(profile)}`}
    >
      <span aria-hidden="true">→</span>
      <span>{textTranslationDisplayName(profile)}</span>
    </span>
  );
}

export function CredentialBadge({ state, nativeSpeech = false }: { state: CredentialState; nativeSpeech?: boolean }) {
  const label = credentialStateText(state, nativeSpeech);
  return <span className="credential-badge" data-state={state} aria-label={label}>{label}</span>;
}
