import { useState, type KeyboardEvent } from "react";
import { appOpenAudioPrivacySettings } from "../../lib/ipc";
import { diagnosticPlatform } from "../../lib/connectionDiagnostics";
import { systemAudioPermissionCopy } from "../../lib/systemAudioPermissions";
import { I18N } from "../../lib/i18n";
import { useAppleSpeechSupport } from "./useAppleSpeechSupport";

/** Re-openable help only; every session and credential action remains explicit. */
export function QuickStartGuide({
  onConfigureService,
  onOpenSubtitles,
}: {
  onConfigureService: () => void;
  onOpenSubtitles: () => void;
}) {
  const permission = systemAudioPermissionCopy();
  const isMac = diagnosticPlatform() === "macos";
  const apple = useAppleSpeechSupport(isMac);
  const [activeTab, setActiveTab] = useState<"setup" | "operations">("setup");
  const tabs = [
    { id: "setup", label: I18N.settings.quickStartSetupTab },
    { id: "operations", label: I18N.settings.quickStartOperationsTitle },
  ] as const;
  const switchTabWithKeyboard = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.nativeEvent.isComposing) return;
    const next = event.key === "Home" ? "setup" : event.key === "End" ? "operations"
      : event.key === "ArrowLeft" || event.key === "ArrowRight" ? activeTab === "setup" ? "operations" : "setup"
      : null;
    if (!next) return;
    event.preventDefault();
    setActiveTab(next);
    event.currentTarget.parentElement?.querySelector<HTMLButtonElement>(`#quick-start-tab-${next}`)?.focus();
  };
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
      <div className="quick-start-tabs" role="tablist" aria-label={I18N.settings.quickStartTitle}>
        {tabs.map(tab => <button
          key={tab.id}
          type="button"
          role="tab"
          id={`quick-start-tab-${tab.id}`}
          aria-controls={`quick-start-panel-${tab.id}`}
          aria-selected={activeTab === tab.id}
          tabIndex={activeTab === tab.id ? 0 : -1}
          onClick={() => setActiveTab(tab.id)}
          onKeyDown={switchTabWithKeyboard}
        >{tab.label}</button>)}
      </div>
      <div id="quick-start-panel-setup" role="tabpanel" aria-labelledby="quick-start-tab-setup" tabIndex={0} hidden={activeTab !== "setup"}>
        <ol className="quick-start-guide">
          <li>
            <span className="quick-start-guide__number" aria-hidden="true">1</span>
            <div>
              <h2>{I18N.settings.quickStartServiceTitle}</h2>
              <p>{I18N.settings.quickStartServiceBody}</p>
              {isMac && !apple.loading && apple.support?.available && <p>{I18N.settings.quickStartAppleBody}</p>}
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
              {isMac && <>
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
      </div>
      <div id="quick-start-panel-operations" className="quick-start-operations" role="tabpanel" aria-labelledby="quick-start-tab-operations" tabIndex={0} hidden={activeTab !== "operations"}>
        <dl className="quick-start-actions">
          <div className="quick-start-operation--move">
            <dt>{I18N.settings.quickStartMoveTitle}</dt>
            <dd>
              <span>{I18N.settings.quickStartMoveBody}</span>
              <svg className="quick-start-window-diagram" viewBox="0 0 144 88" aria-hidden="true" focusable="false">
                <rect x="1" y="1" width="142" height="86" rx="11" fill="#252525" stroke="#555" />
                <rect x="54" y="11" width="36" height="3" rx="1.5" fill="#fff" />
                <rect x="18" y="39" width="108" height="4" rx="2" fill="#aaa" />
                <rect x="30" y="54" width="84" height="3" rx="1.5" fill="#777" />
                <path d="M80 13v22l6-6 5 10 5-3-5-9h10Z" fill="#fff" stroke="#252525" strokeWidth="1.5" strokeLinejoin="round" />
              </svg>
            </dd>
          </div>
          <div>
            <dt>{I18N.settings.quickStartResizeTitle}</dt>
            <dd>{I18N.settings.quickStartResizeBody}</dd>
          </div>
          <div>
            <dt>{I18N.settings.quickStartCollapseTitle}</dt>
            <dd>{I18N.settings.quickStartCollapseBody}</dd>
          </div>
          <div>
            <dt>{I18N.settings.lockPosition}</dt>
            <dd>{I18N.settings.quickStartLockBody}</dd>
          </div>
          <div>
            <dt>{I18N.settings.blendBackground}</dt>
            <dd>{I18N.settings.quickStartImmersiveBody}</dd>
          </div>
        </dl>
      </div>
    </section>
  );
}
