import { useState } from "react";
import { Icon } from "../../../../components/Icon";
import { ProviderIcon } from "../../../../components/providers/ProviderIcon";
import { I18N, providerDisplayName } from "../../../../lib/i18n";
import { SERVICE_PROVIDERS, isCustomSpeechProvider } from "../../../../lib/providers/providerCapabilities";
import type { ServiceProvider } from "../../../../lib/types";
import { SettingsConfirmation } from "../../controls/DestructiveConfirmation";
import { SettingsHelp } from "../../feedback/SettingsHelp";
import { InlineFeedback } from "../../controls/SettingsPrimitives";
import type { useAppleSpeechSupport } from "../providers/apple/useAppleSpeechSupport";
import { providerDescription } from "./serviceProfilePresentation";

/** A chosen provider stays local until the explicit Add confirmation. */
export function ProviderPicker(
  {
    initialSetup,
    apple,
    disabled,
    cancelDisabled,
    requiresStop,
    feedback,
    onChoose,
    onCancel,
  }: {
    initialSetup: boolean;
    apple: ReturnType<typeof useAppleSpeechSupport>;
    disabled: boolean;
    cancelDisabled: boolean;
    requiresStop: boolean;
    feedback: { tone: "success" | "error" | "info"; message: string } | null;
    onChoose: (provider: ServiceProvider) => void;
    onCancel: () => void;
  }
) {
  const [selectedProvider, setSelectedProvider] = useState<ServiceProvider | null>(null);
  return (
    <div className="provider-picker settings-panel" data-initial-setup={initialSetup}>
      <div className="provider-picker__heading">
        <span>
          <h2 tabIndex={-1} data-service-page-heading>
            {initialSetup ? I18N.settings.firstProfileTitle : I18N.settings.chooseProvider}
          </h2>
        </span>
        {!initialSetup && (
          <button type="button" className="settings-link" disabled={cancelDisabled} onClick={onCancel}>
            {I18N.settings.cancel}
          </button>
        )}
      </div>
      {!selectedProvider && requiresStop
        && <InlineFeedback tone="info">{I18N.settings.profileCreateRequiresStop}</InlineFeedback>}
      {!selectedProvider && feedback
        && <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>}
      {apple.failed
        && <InlineFeedback tone="error">{I18N.settings.appleSpeechLoadFailed} <button
          type="button"
          className="settings-link"
          disabled={disabled || apple.loading}
          onClick={() => void apple.refresh()}
        >{I18N.settings.retryLoadingSettings}</button></InlineFeedback>}
      <div className="provider-picker__options">
        {SERVICE_PROVIDERS.filter(provider => provider !== "appleSpeech" || apple.support?.available).map(
          (provider) => (
            <div className="provider-picker__option-row" key={provider}>
              <button
                type="button"
                className="provider-option"
                data-provider={provider}
                disabled={disabled}
                onClick={() => setSelectedProvider(provider)}
              >
                <ProviderIcon provider={provider} size={24} />
                <span><strong>{providerDisplayName(provider)}</strong></span>
                <Icon name="chevron-right" />
              </button>
              <SettingsHelp
                text={isCustomSpeechProvider(provider)
                  ? I18N.settings.customSpeechDescription
                  : providerDescription(provider)}
                label={`${providerDisplayName(provider)}: ${I18N.settings.helpLabel}`}
              />
            </div>
          )
        )}
      </div>
      {selectedProvider
        && <SettingsConfirmation
          message={I18N.settings.confirmAddProfile}
          disabled={disabled}
          variant="default"
          confirmLabel={I18N.settings.confirmAddProfile}
          confirmIcon="plus"
          onCancel={() => { if(!cancelDisabled) setSelectedProvider(null); }}
          onConfirm={() => onChoose(selectedProvider)}
        >
          <div className="provider-picker__preview">
            <ProviderIcon provider={selectedProvider} />
            <div className="provider-picker__name-help">
              <h3>{providerDisplayName(selectedProvider)}</h3>
              <SettingsHelp text={providerDescription(selectedProvider)} label={I18N.settings.helpLabel} />
            </div>
          </div>
          {requiresStop
            && <InlineFeedback tone="info">{I18N.settings.profileCreateRequiresStop}</InlineFeedback>}
          {feedback && <InlineFeedback tone={feedback.tone}>{feedback.message}</InlineFeedback>}
        </SettingsConfirmation>}
    </div>
  );
}
