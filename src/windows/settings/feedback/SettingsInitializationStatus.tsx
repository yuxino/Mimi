import { I18N } from "../../../lib/i18n";
import { InlineFeedback } from "../controls/SettingsPrimitives";

export function SettingsInitializationStatus({ status, error, onRetry }: {
  status: "idle" | "loading" | "ready" | "error";
  error: "timeout" | "unavailable" | null;
  onRetry: () => void;
}) {
  return <div className="settings-initialization" aria-busy={status === "loading" || status === "idle"}>
    <InlineFeedback tone={status === "error" ? "error" : "info"}>
      {status === "error" ? error === "timeout" ? I18N.settings.settingsSnapshotTimeout : I18N.settings.settingsSnapshotFailed : I18N.settings.settingsSnapshotLoading}
    </InlineFeedback>
    {status === "error" && <button type="button" className="settings-button settings-button--quiet" onClick={onRetry}>{I18N.settings.retryLoadingSettings}</button>}
  </div>;
}
