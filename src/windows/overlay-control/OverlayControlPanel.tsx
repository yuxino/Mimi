import { SettingsHelp } from "../settings/SettingsHelp";
import { useDesktopShortcuts } from "../../lib/useDesktopShortcuts";
import { Select } from "../../components/Select";
import { LanguageSelect } from "../../components/LanguageSelect";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import {
  isTauri,
  overlayControlSetPanelHeight,
  type SettingsNavigationTarget,
} from "../../lib/ipc";
import { SUBTITLE_DISPLAY_OPTIONS, subtitleDisplayShortcut } from "../../lib/subtitleDisplay";
import type { SubtitleDisplayMode } from "../../lib/types";
import {
  SOURCE_LANGUAGE_DISPLAY_NAMES,
  type OverlayActivityPhaseKind,
  type SettingsSnapshot,
  type SourceLanguage,
} from "../../lib/types";
import {
  type LanguageStatus,
} from "../overlay/overlayModel";
import { LanguageStatusCapsule } from "./LanguageStatusCapsule";
import { CaptureStatusRow } from "./CaptureStatusRow";
import type { OverlayControlPanelModel } from "./overlayControlModel";

type PendingAction =
  | "pause"
  | "display"
  | "source"
  | "translation"
  | "intermediate"
  | "timestamps"
  | "immersive"
  | "lock"
  | "settings";

interface OverlayControlPanelProps {
  phase: OverlayActivityPhaseKind;
  status: LanguageStatus;
  settings: SettingsSnapshot;
  model: OverlayControlPanelModel;
  isPaused: boolean;
  canPauseSession: boolean;
  isWaitingForFinalTranslation: boolean;
  isChangingSession: boolean;
  isStopping?: boolean;
  onDismiss: () => void;
  onTogglePaused: () => Promise<void>;
  onSwitchSourceLanguage: (language: SourceLanguage) => Promise<void>;
  onSetSkipTranslation: (enabled: boolean) => Promise<void>;
  onSetIntermediateSubtitles: (enabled: boolean) => Promise<void>;
  onSetSubtitleTimestamps: (enabled: boolean) => Promise<void>;
  onSetSubtitleDisplayMode: (mode: SubtitleDisplayMode) => Promise<void>;
  onSetImmersiveMode: (enabled: boolean) => Promise<void>;
  onSetOverlayLocked: (locked: boolean) => Promise<void>;
  onShowSettings: (target?: SettingsNavigationTarget) => Promise<void>;
}

export function OverlayControlPanel({
  phase,
  status,
  settings,
  model,
  isPaused,
  canPauseSession,
  isWaitingForFinalTranslation,
  isChangingSession,
  isStopping = false,
  onDismiss,
  onTogglePaused,
  onSwitchSourceLanguage,
  onSetSkipTranslation,
  onSetIntermediateSubtitles,
  onSetSubtitleTimestamps,
  onSetSubtitleDisplayMode,
  onSetImmersiveMode,
  onSetOverlayLocked,
  onShowSettings,
}: OverlayControlPanelProps) {
  const { nativeShortcuts } = useDesktopShortcuts();
  const panelRef = useRef<HTMLElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const sourceControlRef = useRef<HTMLDivElement>(null);
  const displayControlRef = useRef<HTMLDivElement>(null);
  const immersiveRef = useRef<HTMLButtonElement>(null);
  const lockRef = useRef<HTMLButtonElement>(null);
  const intermediateHelpId = useId();
  const timestampsHelpId = useId();
  const actionInFlight = useRef(false);
  const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const canChangeSessionSettings = !isChangingSession && pendingAction === null;

  useLayoutEffect(() => {
    if (!isTauri || !panelRef.current || !contentRef.current) return;
    let animationFrame = 0;
    let lastHeight = 0;
    const measure = () => {
      animationFrame = 0;
      const panel = panelRef.current;
      const content = contentRef.current;
      if (!panel || !content) return;
      const height = Math.ceil(
        content.getBoundingClientRect().height + panel.clientTop * 2,
      );
      if (height === lastHeight) return;
      lastHeight = height;
      void overlayControlSetPanelHeight(height).catch(() => {});
    };
    const scheduleMeasure = () => {
      if (animationFrame !== 0) return;
      animationFrame = window.requestAnimationFrame(measure);
    };
    const observer = new ResizeObserver(scheduleMeasure);
    observer.observe(contentRef.current);
    scheduleMeasure();
    return () => {
      observer.disconnect();
      if (animationFrame !== 0) window.cancelAnimationFrame(animationFrame);
    };
  }, []);

  useEffect(() => {
    const target = [
      sourceControlRef.current?.querySelector<HTMLButtonElement>('[role="combobox"]'),
      displayControlRef.current?.querySelector<HTMLButtonElement>('[role="combobox"]'),
      immersiveRef.current,
      lockRef.current,
    ].find((candidate) => candidate != null && !candidate.disabled);
    const animationFrame = window.requestAnimationFrame(() => target?.focus());
    return () => window.cancelAnimationFrame(animationFrame);
  }, []);

  const performAction = (
    name: PendingAction,
    operation: () => Promise<void>,
    dismissAfter = true,
    failureMessage = I18N.overlay.controlActionFailed,
  ) => {
    if (actionInFlight.current) return;
    actionInFlight.current = true;
    setPendingAction(name);
    setOperationError(null);
    void operation()
      .then(() => {
        if (dismissAfter) onDismiss();
      })
      .catch(() => setOperationError(failureMessage))
      .finally(() => { actionInFlight.current = false; setPendingAction(null); });
  };

  return (
    <section
      ref={panelRef}
      id="overlay-control-panel"
      className="overlay-control-panel"
      role="dialog"
      aria-modal="false"
      aria-label={I18N.overlay.controlPanel}
      aria-busy={pendingAction !== null}
    >
      <div ref={contentRef} className="overlay-control-panel__content">
        <LanguageStatusCapsule
          phase={phase}
          status={status}
          settings={settings}
          isPaused={isPaused}
          isWaitingForFinalTranslation={isWaitingForFinalTranslation}
          expanded
          isStopping={isStopping}
          onToggle={onDismiss}
        />

        <button
          type="button"
          className="overlay-control-session-action"
          aria-label={isPaused ? I18N.overlay.resume : I18N.overlay.pause}
          disabled={!canPauseSession || isChangingSession || pendingAction !== null}
          onClick={() => performAction("pause", onTogglePaused, false)}
        >
          <Icon name={isPaused ? "play" : "pause"} />
          <span>{isPaused ? I18N.overlay.resume : I18N.overlay.pause}</span>
        </button>

        <CaptureStatusRow
          disabled={pendingAction !== null}
        />

        <div ref={displayControlRef} className="overlay-control-picker" title={nativeShortcuts ? subtitleDisplayShortcut() : undefined}>
          <span>{I18N.settings.subtitleDisplay}</span>
          <Select label={I18N.settings.subtitleDisplay} value={settings.subtitleDisplayMode}
            options={SUBTITLE_DISPLAY_OPTIONS} disabled={pendingAction !== null}
            onChange={(value) => performAction("display", () => onSetSubtitleDisplayMode(value as SubtitleDisplayMode), false)} />
        </div>

        {model.sourceOptions.length > 0 && (
          <div ref={sourceControlRef} className="overlay-control-picker">
            <span>{I18N.overlay.sourceLanguage} <SettingsHelp text={I18N.settings.recognitionLanguageHelp} label={I18N.settings.helpLabel} /></span>
            <LanguageSelect
              label={I18N.overlay.sourceLanguage}
              value={settings.sourceLanguage}
              options={model.sourceOptions.map((language) => ({
                value: language,
                label: SOURCE_LANGUAGE_DISPLAY_NAMES[language],
              }))}
              disabled={!canChangeSessionSettings}
              onChange={(value) => performAction("source", () => onSwitchSourceLanguage(value as SourceLanguage))}
            />
          </div>
        )}

        <div className="overlay-control-divider" />

        {model.canSkipTranslation && <button
          type="button"
          role="switch"
          aria-checked={settings.targetLanguage === "original"}
          aria-label={I18N.settings.skipTranslation}
          className={`overlay-control-setting${settings.targetLanguage === "original" ? " is-on" : ""}`}
          disabled={!canChangeSessionSettings}
          onClick={() => performAction("translation", () => onSetSkipTranslation(settings.targetLanguage !== "original"), false)}
        >
          <span className="overlay-control-setting__icon" aria-hidden="true"><Icon name="languages" /></span>
          <span className="overlay-control-setting__copy"><strong>{I18N.settings.skipTranslation}</strong></span>
          <span className="overlay-control-switch" aria-hidden="true"><span /></span>
        </button>}

        <div className="overlay-control-setting-row">
          <span className="overlay-control-setting__icon" aria-hidden="true"><Icon name="captions-bubble" /></span>
          <span className="overlay-control-setting__copy">
            <strong>{I18N.settings.showIntermediateSubtitles}</strong>
            <SettingsHelp id={intermediateHelpId} text={I18N.settings.showIntermediateSubtitlesHelp} label={I18N.settings.helpLabel} />
          </span>
          <button type="button" role="switch"
            aria-checked={settings.showIntermediateSubtitles !== false}
            aria-label={I18N.settings.showIntermediateSubtitles}
            aria-describedby={intermediateHelpId}
            className={`overlay-control-setting overlay-control-setting--toggle${settings.showIntermediateSubtitles !== false ? " is-on" : ""}`}
            disabled={pendingAction !== null}
            onClick={() => performAction("intermediate", () => onSetIntermediateSubtitles(settings.showIntermediateSubtitles === false), false)}
          >
            <span className="overlay-control-switch" aria-hidden="true"><span /></span>
          </button>
        </div>

        <div className="overlay-control-setting-row">
          <span className="overlay-control-setting__icon" aria-hidden="true"><Icon name="captions-bubble" /></span>
          <span className="overlay-control-setting__copy">
            <strong>{I18N.settings.subtitleTimestamps}</strong>
            <SettingsHelp id={timestampsHelpId} text={I18N.settings.subtitleTimestampsHelp} label={I18N.settings.helpLabel} />
          </span>
          <button type="button" role="switch"
            aria-checked={settings.showSubtitleTimestamps ?? false}
            aria-label={I18N.settings.subtitleTimestamps}
            aria-describedby={timestampsHelpId}
            className={`overlay-control-setting overlay-control-setting--toggle${settings.showSubtitleTimestamps ? " is-on" : ""}`}
            disabled={pendingAction !== null}
            onClick={() => performAction("timestamps", () => onSetSubtitleTimestamps(!settings.showSubtitleTimestamps), false,
              I18N.settings.settingSaveFailed(I18N.settings.subtitleTimestamps))}
          >
            <span className="overlay-control-switch" aria-hidden="true"><span /></span>
          </button>
        </div>

        <button
          ref={immersiveRef}
          type="button"
          role="switch"
          aria-checked={model.immersiveModeEnabled}
          aria-label={I18N.overlay.immersiveMode}
          className={`overlay-control-setting${model.immersiveModeEnabled ? " is-on" : ""}`}
          disabled={pendingAction !== null}
          onClick={() =>
            performAction("immersive", () =>
              onSetImmersiveMode(!model.immersiveModeEnabled),
            )
          }
        >
          <span className="overlay-control-setting__icon" aria-hidden="true">
            <Icon name="blend" />
          </span>
          <span className="overlay-control-setting__copy">
            <strong>{I18N.overlay.immersiveMode}</strong>
          </span>
          <span className="overlay-control-switch" aria-hidden="true">
            <span />
          </span>
        </button>

        <button
          ref={lockRef}
          type="button"
          role="switch"
          aria-checked={model.overlayLocked}
          aria-label={I18N.overlay.lockPosition}
          className={`overlay-control-setting${model.overlayLocked ? " is-on" : ""}`}
          disabled={pendingAction !== null}
          onClick={() =>
            performAction("lock", () =>
              onSetOverlayLocked(!model.overlayLocked),
            )
          }
        >
          <span className="overlay-control-setting__icon" aria-hidden="true">
            <Icon name={model.overlayLocked ? "unlock" : "lock"} />
          </span>
          <span className="overlay-control-setting__copy">
            <strong>
              {model.overlayLocked
                ? I18N.overlay.unlockPosition
                : I18N.overlay.lockPosition}
            </strong>
          </span>
          <span className="overlay-control-switch" aria-hidden="true">
            <span />
          </span>
        </button>

        <button
          type="button"
          className="overlay-control-settings-link"
          disabled={pendingAction !== null}
          onClick={() => performAction("settings", onShowSettings, false)}
        >
          <Icon name="gear" />
          <span>{I18N.overlay.moreSettings}</span>
        </button>

        {operationError && (
          <div className="overlay-control-alert" role="alert">
            <Icon name="exclamation-triangle" />
            <span>{operationError}</span>
          </div>
        )}
      </div>
    </section>
  );
}
