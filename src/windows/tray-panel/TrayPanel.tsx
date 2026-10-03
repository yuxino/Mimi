import { SettingsHelp } from "../settings/SettingsHelp";
import { useDesktopShortcuts } from "../../lib/useDesktopShortcuts";
import { Select } from "../../components/Select";
import { LanguageSelect } from "../../components/LanguageSelect";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Icon, type IconName } from "../../components/Icon";
import { I18N, providerDisplayName } from "../../lib/i18n";
import { isTauri } from "../../lib/ipc";
import {
  activeServiceProfile,
  credentialStateForTarget,
  sourceLanguagesForSettings,
} from "../../lib/providerCapabilities";
import {
  selectSessionErrorMessage,
  selectSessionStatusKind,
  useStore,
} from "../../lib/store";
import {
  SOURCE_LANGUAGE_DISPLAY_NAMES,
  TARGET_LANGUAGE_DISPLAY_NAMES,
  targetLanguageTranslatesAudio,
  type SettingsSnapshot,
  type SourceLanguage,
  type SubtitleAlignment,
} from "../../lib/types";
import { subtitleDisplayShortcut } from "../../lib/subtitleDisplay";
import type { SubtitleDisplayMode } from "../../lib/types";
import {
  actionErrorMessage,
  deriveTrayPresentation,
  hasSubtitleContent,
  type TrayActionPresentation,
  type TraySessionAction,
  type TrayStatusKind,
} from "./trayModel";
import "./tray-panel.css";

const TRAY_WINDOW_WIDTH = 320;
const TRAY_WINDOW_PADDING = 6;

type PendingAction =
  | TraySessionAction
  | "language"
  | "display"
  | "alignment"
  | "blend"
  | "lock"
  | "dock"
  | "show"
  | "clear"
  | "settings"
  | "quit";

/** Compact cross-platform command center shown from the tray icon. */
export function TrayPanel() {
  const { nativeShortcuts } = useDesktopShortcuts();
  // Keep streaming subtitle updates from repainting this hidden native window:
  // each selector returns only the primitive state rendered by the tray.
  const sessionStatusKind = useStore(selectSessionStatusKind);
  const sessionErrorMessage = useStore(selectSessionErrorMessage);
  const isPaused = useStore((state) => state.session.isPaused);
  const subtitleHasContent = useStore((state) =>
    hasSubtitleContent(state.session.subtitles),
  );
  const settings = useStore((state) => state.settings);
  const start = useStore((state) => state.start);
  const stop = useStore((state) => state.stop);
  const togglePaused = useStore((state) => state.togglePaused);
  const switchSourceLanguage = useStore((state) => state.switchSourceLanguage);
  const saveSettings = useStore((state) => state.saveSettings);
  const setOverlayLocked = useStore((state) => state.setOverlayLocked);
  const showOverlay = useStore((state) => state.showOverlay);
  const clearSubtitles = useStore((state) => state.clearSubtitles);
  const hideTrayPanel = useStore((state) => state.hideTrayPanel);
  const showSettings = useStore((state) => state.showSettings);
  const quit = useStore((state) => state.quit);

  const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const operationPending = useRef(false);
  const panelRef = useRef<HTMLElement>(null);

  const activeProfile = activeServiceProfile(settings);
  const sourceLanguages = sourceLanguagesForSettings(settings);
  const presentation = deriveTrayPresentation({
    statusKind: sessionStatusKind,
    isPaused,
    credentialState: credentialStateForTarget(activeProfile, settings.targetLanguage),
    hasSubtitleContent: subtitleHasContent,
  });
  const anyActionPending = pendingAction !== null;
  const isMacOS =
    typeof navigator !== "undefined" &&
    /Macintosh|MacIntel/i.test(navigator.userAgent + " " + navigator.platform);
  const sourcePickerDisabled =
    anyActionPending ||
    !presentation.canChangeSourceLanguage ||
    sourceLanguages.length === 1;

  const performAction = (
    name: PendingAction,
    operation: () => Promise<void>,
  ) => {
    if (operationPending.current) return;
    operationPending.current = true;
    setPendingAction(name);
    setOperationError(null);
    void operation()
      .catch((error: unknown) => {
        setOperationError(
          name === "quit"
            ? I18N.tray.quitFailed
            : name === "dock"
              ? I18N.settings.dockSaveFailed
              : actionErrorMessage(error, I18N.settings.profileActionFailed),
        );
      })
      .finally(() => {
        operationPending.current = false;
        setPendingAction(null);
      });
  };

  const runSessionAction = (action: TraySessionAction) => {
    switch (action) {
      case "start":
        performAction(action, start);
        break;
      case "configure":
        performAction(action, () => showSettings("service"));
        break;
      case "pause":
      case "resume":
        performAction(action, togglePaused);
        break;
      case "stop":
        performAction(action, stop);
        break;
      case "connecting":
      case "stopping":
        break;
    }
  };

  // Match the native window to the rendered surface so inline errors and
  // localized labels never leave a transparent click-catching tail.
  useLayoutEffect(() => {
    if (!isTauri || !panelRef.current) return;

    const currentWindow = getCurrentWindow();
    let lastHeight = 0;
    let animationFrame = 0;
    const resize = () => {
      animationFrame = 0;
      const surface = panelRef.current;
      if (!surface) return;
      const height = Math.ceil(
        surface.getBoundingClientRect().height + TRAY_WINDOW_PADDING * 2,
      );
      if (height === lastHeight) return;
      lastHeight = height;
      void currentWindow
        .setSize(new LogicalSize(TRAY_WINDOW_WIDTH, height))
        .catch(() => {});
    };
    const scheduleResize = () => {
      if (animationFrame !== 0) return;
      animationFrame = window.requestAnimationFrame(resize);
    };
    const observer = new ResizeObserver(scheduleResize);
    observer.observe(panelRef.current);
    scheduleResize();

    return () => {
      observer.disconnect();
      if (animationFrame !== 0) window.cancelAnimationFrame(animationFrame);
    };
  }, []);

  useEffect(() => {
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      void hideTrayPanel().catch((error: unknown) => {
        setOperationError(
          actionErrorMessage(error, I18N.settings.profileActionFailed),
        );
      });
    };
    window.addEventListener("keydown", dismissOnEscape);
    return () => window.removeEventListener("keydown", dismissOnEscape);
  }, [hideTrayPanel]);

  const panel = (
    <section
      ref={panelRef}
      className="tray-panel"
      data-state={presentation.visualState}
      aria-label={I18N.tray.appName}
    >
      <header className="tray-header">
        <span className="tray-wordmark" aria-hidden="true">
          <Icon name="captions-bubble" />
        </span>

        <span className="tray-header__copy">
          <strong>{I18N.tray.appName}</strong>
          <span className="tray-status" aria-live="polite">
            <span className="tray-status__dot" aria-hidden="true" />
            <span>
              {statusText(presentation.statusKind, sessionErrorMessage)}
            </span>
          </span>
        </span>

        <span
          className="tray-profile"
          title={
            activeProfile
              ? `${activeProfile.name} · ${providerDisplayName(activeProfile.provider)}`
              : I18N.settings.noActiveProfile
          }
          aria-label={`${I18N.settings.currentProfile}: ${activeProfile?.name ?? I18N.settings.noActiveProfile}`}
        >
          <Icon
            name={
              activeProfile?.provider === "xAIRealtime"
                ? "waves"
                : activeProfile?.provider === "alibabaCloud" ||
                    activeProfile?.provider === "azureOpenAIRealtime"
                  ? "cloud"
                  : "languages"
            }
          />
          <span>{activeProfile?.name ?? I18N.settings.noActiveProfile}</span>
        </span>
      </header>

      <div
        className="tray-session-actions"
        data-layout={presentation.secondaryAction ? "split" : "single"}
      >
        <SessionActionButton
          presentation={presentation.primaryAction}
          pending={pendingAction === presentation.primaryAction.action}
          blocked={anyActionPending}
          onClick={runSessionAction}
        />
        {presentation.secondaryAction && (
          <SessionActionButton
            presentation={presentation.secondaryAction}
            pending={pendingAction === presentation.secondaryAction.action}
            blocked={anyActionPending}
            onClick={runSessionAction}
            secondary
          />
        )}
      </div>

      <div className="tray-card" aria-label={I18N.settings.subtitleTitle}>
        <div className="tray-setting-row tray-setting-row--language">
          <span className="tray-setting-row__icon" aria-hidden="true">
            <Icon name="languages" />
          </span>
          <span className="tray-setting-row__copy">
            <span>{I18N.tray.sourceLanguage} <SettingsHelp text={I18N.settings.recognitionLanguageHelp} label={I18N.settings.helpLabel} /></span>
            <small>{translationSummary(settings)}</small>
          </span>
          <span className="tray-select-wrap">
            <LanguageSelect label={I18N.tray.sourceLanguage} value={settings.sourceLanguage}
              disabled={sourcePickerDisabled}
              options={sourceLanguages.map((language) => ({ value: language, label: SOURCE_LANGUAGE_DISPLAY_NAMES[language] }))}
              onChange={(value) => performAction("language", () => switchSourceLanguage(value as SourceLanguage))} />
          </span>
        </div>

        <span className="tray-card__divider" />

        <div className="tray-setting-row tray-setting-row--display" title={nativeShortcuts ? subtitleDisplayShortcut() : undefined}>
          <span className="tray-setting-row__icon" aria-hidden="true"><Icon name="captions-bubble" /></span>
          <span className="tray-setting-row__copy"><span>{I18N.settings.subtitleDisplay}</span></span>
          <span className="tray-select-wrap">
            <Select label={I18N.settings.subtitleDisplay} value={settings.subtitleDisplayMode}
              options={[
                { value: "translation", label: I18N.tray.displayTranslation },
                { value: "bilingual", label: I18N.tray.displayBilingual },
                { value: "original", label: I18N.tray.displayOriginal },
              ]} disabled={anyActionPending}
              onChange={(value) => performAction("display", () => saveSettings({ subtitleDisplayMode: value as SubtitleDisplayMode }))} />
          </span>
        </div>
        <span className="tray-card__divider" />

        <div className="tray-setting-row tray-setting-row--alignment">
          <span className="tray-setting-row__icon" aria-hidden="true">
            <Icon name="align-center" />
          </span>
          <span className="tray-setting-row__copy">
            <span>{I18N.tray.subtitleAlignment}</span>
          </span>
          <TrayAlignmentControl
            value={settings.subtitleAlignment}
            disabled={anyActionPending}
            onChange={(subtitleAlignment) =>
              performAction("alignment", () =>
                saveSettings({ subtitleAlignment }),
              )
            }
          />
        </div>

        <span className="tray-card__divider" />

        <button
          type="button"
          role="switch"
          aria-checked={settings.subtitleBlendsWithBackground}
          aria-label={I18N.tray.blendBackground}
          disabled={anyActionPending}
          className={`tray-setting-row tray-setting-row--toggle${settings.subtitleBlendsWithBackground ? " is-checked" : ""}`}
          onClick={() =>
            performAction("blend", () =>
              saveSettings({
                subtitleBlendsWithBackground:
                  !settings.subtitleBlendsWithBackground,
              }),
            )
          }
        >
          <span className="tray-setting-row__icon" aria-hidden="true">
            <Icon name="blend" />
          </span>
          <span className="tray-setting-row__copy">
            <span>{I18N.tray.blendBackground}</span>
          </span>
          <span className="tray-switch" aria-hidden="true">
            <span />
          </span>
        </button>

        <span className="tray-card__divider" />

        <button
          type="button"
          role="switch"
          aria-checked={settings.isOverlayLocked}
          aria-label={I18N.tray.lockPosition}
          disabled={anyActionPending}
          className={`tray-setting-row tray-setting-row--toggle${settings.isOverlayLocked ? " is-checked" : ""}`}
          onClick={() =>
            performAction("lock", () =>
              setOverlayLocked(!settings.isOverlayLocked),
            )
          }
        >
          <span className="tray-setting-row__icon" aria-hidden="true">
            <Icon name="lock" />
          </span>
          <span className="tray-setting-row__copy">
            <span>{I18N.tray.lockPosition}</span>
          </span>
          <span className="tray-switch" aria-hidden="true">
            <span />
          </span>
        </button>

        {isMacOS && (
          <>
            <span className="tray-card__divider" />
            <button
              type="button"
              role="switch"
              aria-checked={settings.showInDock}
              aria-label={I18N.settings.showInDock}
              aria-busy={pendingAction === "dock"}
              disabled={anyActionPending}
              className={`tray-setting-row tray-setting-row--toggle${settings.showInDock ? " is-checked" : ""}`}
              onClick={() =>
                performAction("dock", () =>
                  saveSettings({ showInDock: !settings.showInDock }),
                )
              }
            >
              <span className="tray-setting-row__icon" aria-hidden="true">
                <Icon name="app-window" />
              </span>
              <span className="tray-setting-row__copy">
                <span>{I18N.settings.showInDock}</span>
              </span>
              <span className="tray-switch" aria-hidden="true">
                <span />
              </span>
            </button>
          </>
        )}
      </div>

      {(presentation.canShowOverlay || presentation.canClearSubtitles) && (
        <div className="tray-tools">
          <ToolButton
            icon="app-window"
            label={I18N.tray.showSubtitleWindow}
            disabled={anyActionPending || !presentation.canShowOverlay}
            pending={pendingAction === "show"}
            onClick={() => performAction("show", showOverlay)}
          />
          <ToolButton
            icon="eraser"
            label={I18N.tray.clearSubtitles}
            disabled={anyActionPending || !presentation.canClearSubtitles}
            pending={pendingAction === "clear"}
            onClick={() => performAction("clear", clearSubtitles)}
          />
        </div>
      )}

      {operationError && (
        <div className="tray-alert" role="alert">
          <Icon name="exclamation-triangle" />
          <span>{operationError}</span>
        </div>
      )}

      <footer className="tray-footer">
        <button
          type="button"
          disabled={anyActionPending}
          onClick={() => performAction("settings", showSettings)}
        >
          <Icon name="gear" />
          <span>{I18N.tray.settings}</span>
        </button>
        <button
          type="button"
          disabled={anyActionPending}
          aria-busy={pendingAction === "quit"}
          data-action="quit"
          onClick={() => performAction("quit", quit)}
        >
          {pendingAction === "quit" ? I18N.tray.quitting : I18N.tray.quit}
        </button>
      </footer>
    </section>
  );

  return (
    <div className={isTauri ? "tray-shell" : "tray-preview"}>{panel}</div>
  );
}

const SUBTITLE_ALIGNMENTS: readonly SubtitleAlignment[] = [
  "left",
  "center",
  "right",
];

function TrayAlignmentControl({
  value,
  disabled,
  onChange,
}: {
  value: SubtitleAlignment;
  disabled: boolean;
  onChange: (alignment: SubtitleAlignment) => void;
}) {
  const labels: Record<SubtitleAlignment, string> = {
    left: I18N.tray.alignLeft,
    center: I18N.tray.alignCenter,
    right: I18N.tray.alignRight,
  };

  return (
    <span
      className="tray-alignment-control"
      role="group"
      aria-label={I18N.tray.subtitleAlignment}
    >
      {SUBTITLE_ALIGNMENTS.map((alignment) => (
        <button
          key={alignment}
          type="button"
          disabled={disabled}
          className={value === alignment ? "is-selected" : undefined}
          aria-label={labels[alignment]}
          aria-pressed={value === alignment}
          title={labels[alignment]}
          onClick={() => onChange(alignment)}
        >
          <Icon name={`align-${alignment}`} />
        </button>
      ))}
    </span>
  );
}

function SessionActionButton({
  presentation,
  pending,
  blocked,
  onClick,
  secondary = false,
}: {
  presentation: TrayActionPresentation;
  pending: boolean;
  blocked: boolean;
  onClick: (action: TraySessionAction) => void;
  secondary?: boolean;
}) {
  const disabled = presentation.disabled || blocked;
  return (
    <button
      type="button"
      className="tray-session-button"
      data-action={presentation.action}
      data-secondary={secondary || undefined}
      disabled={disabled}
      aria-busy={pending}
      onClick={() => onClick(presentation.action)}
    >
      {pending || presentation.disabled ? (
        <span className="tray-session-button__spinner" aria-hidden="true" />
      ) : (
        <Icon name={sessionActionIcon(presentation.action)} />
      )}
      <span>{sessionActionLabel(presentation.action)}</span>
    </button>
  );
}

function ToolButton({
  icon,
  label,
  disabled,
  pending,
  onClick,
}: {
  icon: IconName;
  label: string;
  disabled: boolean;
  pending: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="tray-tool-button"
      disabled={disabled}
      aria-busy={pending}
      onClick={onClick}
    >
      <span className="tray-tool-button__icon" aria-hidden="true">
        <Icon name={icon} />
      </span>
      <span>{label}</span>
    </button>
  );
}

function statusText(
  kind: TrayStatusKind,
  sessionErrorMessage: string | null,
): string {
  switch (kind) {
    case "ready":
      return I18N.tray.ready;
    case "setupRequired":
      return I18N.tray.setupRequired;
    case "connecting":
      return I18N.tray.connecting;
    case "listening":
      return I18N.tray.listening;
    case "paused":
      return I18N.tray.paused;
    case "stopping":
      return I18N.tray.stopping;
    case "error":
      return sessionErrorMessage ?? I18N.settings.profileActionFailed;
  }
}

function sessionActionLabel(action: TraySessionAction): string {
  switch (action) {
    case "start":
      return I18N.settings.start;
    case "configure":
      return I18N.settings.configureService;
    case "pause":
      return I18N.overlay.pause;
    case "resume":
      return I18N.overlay.resume;
    case "stop":
      return I18N.settings.stop;
    case "connecting":
      return I18N.tray.connecting;
    case "stopping":
      return I18N.tray.stopping;
  }
}

function sessionActionIcon(action: TraySessionAction): IconName {
  switch (action) {
    case "start":
      return "play";
    case "configure":
      return "key";
    case "pause":
      return "pause";
    case "resume":
      return "play";
    case "stop":
      return "stop";
    case "connecting":
    case "stopping":
      return "waves";
  }
}

function translationSummary(settings: SettingsSnapshot) {
  if (!targetLanguageTranslatesAudio(settings.targetLanguage)) {
    return I18N.tray.originalOnly;
  }
  const target = TARGET_LANGUAGE_DISPLAY_NAMES[settings.targetLanguage];
  return `→ ${target}`;
}
