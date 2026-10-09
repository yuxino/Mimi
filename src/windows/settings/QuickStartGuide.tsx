import { useState } from "react";
import { appOpenAudioPrivacySettings } from "../../lib/ipc";
import { diagnosticPlatform } from "../../lib/connectionDiagnostics";
import { systemAudioPermissionCopy } from "../../lib/systemAudioPermissions";
import { I18N } from "../../lib/i18n";

/** Re-openable help only; every session and credential action remains explicit. */
export function QuickStartGuide({
  onConfigureService,
  onOpenSubtitles,
}: {
  onConfigureService: () => void;
  onOpenSubtitles: () => void;
}) {
  const permission = systemAudioPermissionCopy();
  const [openingPermissions, setOpeningPermissions] = useState(false);
  const [openFailed, setOpenFailed] = useState(false);
  const openPermissions = async () => {
    if (openingPermissions) return;
    setOpeningPermissions(true);
    setOpenFailed(false);
    try { await appOpenAudioPrivacySettings(); }
    catch { setOpenFailed(true); }
    finally { setOpeningPermissions(false); }
  };
  return (
    <section aria-label={I18N.settings.quickStartTitle}>
      <ol className="quick-start-guide">
        <li>
          <span className="quick-start-guide__number" aria-hidden="true">1</span>
          <div>
            <h2>{I18N.settings.quickStartServiceTitle}</h2>
            <p>{I18N.settings.quickStartServiceBody}</p>
            <button type="button" className="settings-button settings-button--quiet settings-button--compact" onClick={onConfigureService}>
              {I18N.settings.configureService}
            </button>
          </div>
        </li>
        <li>
          <span className="quick-start-guide__number" aria-hidden="true">2</span>
          <div>
            <h2>{I18N.settings.quickStartAudioTitle}</h2>
            <p>{I18N.settings.quickStartAudioBody}</p>
            {diagnosticPlatform() === "macos" && <>
              <p>{permission.guide}</p>
              <button type="button" className="settings-button settings-button--quiet settings-button--compact" disabled={openingPermissions} onClick={() => { void openPermissions(); }}>
                {permission.open}
              </button>
              {openFailed && <p className="settings-feedback" data-tone="error" role="alert">{permission.failed}</p>}
            </>}
          </div>
        </li>
        <li>
          <span className="quick-start-guide__number" aria-hidden="true">3</span>
          <div>
            <h2>{I18N.settings.quickStartDisplayTitle}</h2>
            <p>{I18N.settings.quickStartDisplayBody}</p>
            <button type="button" className="settings-button settings-button--quiet settings-button--compact" onClick={onOpenSubtitles}>
              {I18N.settings.quickStartOpenSubtitles}
            </button>
          </div>
        </li>
      </ol>
    </section>
  );
}
