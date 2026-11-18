import { profileSelectionState } from "../../lib/profileSelection";
import { SettingsToastRegion } from "../settings/SettingsToast";
import { activeServiceProfile } from "../../lib/providerCapabilities";
import { speechLanguageGuidance, targetLanguageOptionLabel } from "../../lib/speechLanguageGuidance";
import { SettingsHelp } from "../settings/SettingsHelp";
import { SessionErrorFeedback } from "../../components/SessionErrorFeedback";
import { useDesktopShortcuts } from "../../lib/useDesktopShortcuts";
import { Select } from "../../components/Select";
import { ProviderIcon } from "../../components/ProviderIcon";
import { LanguageSelect } from "../../components/LanguageSelect";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { I18N } from "../../lib/i18n";
import { diagnosticPlatform, languageActionErrorMessage, profileErrorMessage, sessionActionErrorMessage } from "../../lib/connectionDiagnostics";
import {
  isTauri,
  overlayControlSetPanelHeight,
  type SettingsNavigationTarget,
} from "../../lib/ipc";
import { SUBTITLE_DISPLAY_OPTIONS, subtitleDisplayShortcut } from "../../lib/subtitleDisplay";
import type { SubtitleDisplayMode } from "../../lib/types";
import {
  type OverlayActivityPhaseKind,
  type AudioRecordingStatus,
  type SettingsSnapshot,
  type SourceLanguage,
  type TargetLanguage,
} from "../../lib/types";
import {
  type LanguageStatus,
} from "../overlay/overlayModel";
import { LanguageStatusCapsule } from "./LanguageStatusCapsule";
import { CaptureStatusRow } from "./CaptureStatusRow";
import { OverlaySessionDetails } from "./OverlaySessionDetails";
import { startOverlayContentDrag } from "../overlay/overlayContentDrag";
import type { OverlayControlPanelModel } from "./overlayControlModel";

type PendingAction =
  | "retry"
  | "profile"
  | "display"
  | "source"
  | "target"
  | "settings";

interface OverlayControlPanelProps {
  phase: OverlayActivityPhaseKind;
  status: LanguageStatus;
  settings: SettingsSnapshot;
  audioRecording?: AudioRecordingStatus;
  model: OverlayControlPanelModel;
  isPaused: boolean;
  isWaitingForFinalTranslation: boolean;
  isChangingSession: boolean;
  isStopping?: boolean;
  sessionErrorMessage?: string | null;
  permissionRequired?: boolean;
  errorSettingsTarget?: SettingsNavigationTarget | null;
  onRetrySession?: () => Promise<void>;
  onDismiss: () => void;
  onSelectProfile: (profileId: string) => Promise<void>;
  onSwitchSourceLanguage: (language: SourceLanguage) => Promise<void>;
  onSwitchTargetLanguage: (language: TargetLanguage) => Promise<void>;
  onSetSubtitleDisplayMode: (mode: SubtitleDisplayMode) => Promise<void>;
  onShowSettings: (target?: SettingsNavigationTarget) => Promise<void>;
}

export function OverlayControlPanel({
  phase,
  status,
  settings,
  audioRecording,
  model,
  isPaused,
  isWaitingForFinalTranslation,
  isChangingSession,
  isStopping = false,
  sessionErrorMessage,
  permissionRequired = false,
  errorSettingsTarget,
  onRetrySession,
  onDismiss,
  onSelectProfile,
  onSwitchSourceLanguage,
  onSwitchTargetLanguage,
  onSetSubtitleDisplayMode,
  onShowSettings,
}: OverlayControlPanelProps) {
  const { nativeShortcuts } = useDesktopShortcuts();
  const panelRef = useRef<HTMLElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const sourceControlRef = useRef<HTMLDivElement>(null);
  const targetControlRef = useRef<HTMLDivElement>(null);
  const displayControlRef = useRef<HTMLDivElement>(null);
  const subtitlesHeadingId = useId();
  const actionInFlight = useRef(false);
  const mounted = useRef(false);
  const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const sessionControlsBusy = isChangingSession || pendingAction !== null;
  const canChangeSessionSettings = !sessionControlsBusy;
  const canDragPanel = isTauri && !model.overlayLocked && !model.immersiveModeEnabled && canChangeSessionSettings;
  const sourceGuidance = speechLanguageGuidance(settings);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  useLayoutEffect(() => {
    if (!isTauri || !panelRef.current || !contentRef.current) return;
    const windowsResize = diagnosticPlatform() === "windows";
    let active = true;
    let animationFrame = 0;
    let lastHeight = 0;
    let lastPixelRatio = 0;
    let initialResize = true;
    let measurement = 0;
    let resizeAcknowledged = false;
    const revealFocusedControl = () => {
      if (!windowsResize || !initialResize || !resizeAcknowledged) return;
      const panel = panelRef.current;
      const header = panel?.querySelector<HTMLElement>(".overlay-control-header");
      // IPC completion only queues the native resize. Wait for the WebView's
      // viewport to expand beyond its island before allowing focus to scroll.
      if (!panel || !header || window.innerHeight <= header.getBoundingClientRect().height) return;
      initialResize = false;
      const focused = document.activeElement;
      if (focused instanceof HTMLElement && panel.contains(focused)) {
        focused.scrollIntoView({ block: "nearest" });
      }
    };
    const measure = () => {
      animationFrame = 0;
      if (!active) return;
      const panel = panelRef.current;
      const content = contentRef.current;
      if (!panel || !content) return;
      const height = Math.ceil(
        content.getBoundingClientRect().height + panel.clientTop * 2,
      );
      const pixelRatio = windowsResize ? window.devicePixelRatio : 1;
      if (height === lastHeight && pixelRatio === lastPixelRatio) {
        revealFocusedControl();
        return;
      }
      lastHeight = height;
      lastPixelRatio = pixelRatio;
      const currentMeasurement = ++measurement;
      resizeAcknowledged = false;
      void overlayControlSetPanelHeight(height).then(() => {
        if (!active || !windowsResize || currentMeasurement !== measurement) return;
        resizeAcknowledged = true;
        scheduleMeasure();
      }).catch(() => {});
    };
    const scheduleMeasure = () => {
      if (!active || animationFrame !== 0) return;
      animationFrame = window.requestAnimationFrame(measure);
    };
    const observer = new ResizeObserver(scheduleMeasure);
    observer.observe(contentRef.current);
    if (windowsResize) window.addEventListener("resize", scheduleMeasure);
    scheduleMeasure();
    return () => {
      active = false;
      observer.disconnect();
      if (windowsResize) window.removeEventListener("resize", scheduleMeasure);
      if (animationFrame !== 0) window.cancelAnimationFrame(animationFrame);
    };
  }, []);

  useEffect(() => {
    const target = [
      sourceControlRef.current?.querySelector<HTMLButtonElement>('[role="combobox"]'),
      targetControlRef.current?.querySelector<HTMLButtonElement>('[role="combobox"]'),
      displayControlRef.current?.querySelector<HTMLButtonElement>('[aria-pressed="true"]'),
    ].find((candidate) => candidate != null && !candidate.disabled);
    // WebView2 may still have the island viewport while native resize is
    // pending. Other platforms retain their existing focus/scroll behavior.
    const animationFrame = window.requestAnimationFrame(() => {
      if (isTauri && diagnosticPlatform() === "windows") target?.focus({ preventScroll: true });
      else target?.focus();
    });
    return () => window.cancelAnimationFrame(animationFrame);
  }, []);

  const performAction = (
    name: PendingAction,
    operation: () => Promise<void>,
    dismissAfter = true,
  ) => {
    if (actionInFlight.current) return;
    actionInFlight.current = true;
    setPendingAction(name);
    setOperationError(null);
    const failureMessage = I18N.overlay.controlActionFailed;
    void operation()
      .then(() => {
        if (mounted.current && dismissAfter) onDismiss();
      })
      .catch((error: unknown) => {
        if (!mounted.current) return;
        setOperationError(
          name === "profile" ? profileErrorMessage(error)
            : name === "source" || name === "target" ? languageActionErrorMessage(error, failureMessage)
              : name === "retry" ? sessionActionErrorMessage(error, failureMessage) : failureMessage,
        );
      })
      .finally(() => {
        if (!mounted.current) return;
        actionInFlight.current = false;
        setPendingAction(null);
      });
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
      data-drag-enabled={canDragPanel ? "" : undefined}
      onPointerDownCapture={event => startOverlayContentDrag(event, !canDragPanel)}
    >
      <SettingsToastRegion />
      <div
        ref={contentRef}
        className="overlay-control-panel__content"
      >
        <LanguageStatusCapsule
          phase={phase}
          status={status}
          settings={settings}
          audioRecording={audioRecording}
          isPaused={isPaused}
          isWaitingForFinalTranslation={isWaitingForFinalTranslation}
          expanded
          isStopping={isStopping}
          onToggle={onDismiss}
        />

        {sessionErrorMessage && <SessionErrorFeedback permissionRequired={permissionRequired}
          message={sessionErrorMessage}
          configureLabel={errorSettingsTarget === "appleSpeechResources" ? I18N.settings.appleSpeechOpenResources : undefined}
          onConfigure={() => performAction("settings", () => onShowSettings(errorSettingsTarget ?? "service"))}
          onRetry={onRetrySession ? () => performAction("retry", onRetrySession, false) : undefined}
          disabled={pendingAction !== null || isChangingSession}
        />}
        <section className="overlay-control-section" aria-label={I18N.settings.voiceTranslation}>
          <div className="overlay-control-language-pair">
            <div ref={sourceControlRef} className="overlay-control-picker">
              <span className="overlay-control-picker__label">
                <span>{I18N.overlay.sourceLanguage}</span>
                <SettingsHelp text={sourceGuidance.help} label={I18N.settings.helpLabel} />
              </span>
              <LanguageSelect
                label={I18N.overlay.sourceLanguage}
                value={settings.sourceLanguage}
                valueLabel={sourceGuidance.optionLabel(settings.sourceLanguage)}
                options={model.sourceOptions.map(language => ({ value: language, label: sourceGuidance.optionLabel(language) }))}
                disabled={model.sourceOptions.length === 0 || (model.sourceOptions.length === 1 && model.sourceOptions[0] === settings.sourceLanguage)}
                busy={sessionControlsBusy}
                onChange={(value) => performAction("source", () => onSwitchSourceLanguage(value as SourceLanguage), false)}
              />
            </div>

            <div ref={targetControlRef} className="overlay-control-picker">
              <span className="overlay-control-picker__label">
                <span>{I18N.settings.translateTo}</span>
                <SettingsHelp text={I18N.settings.translationConfiguredHelp} label={I18N.settings.helpLabel} />
              </span>
              <LanguageSelect
                label={I18N.settings.translateTo}
                value={settings.targetLanguage}
                valueLabel={targetLanguageOptionLabel(settings, settings.targetLanguage)}
                options={model.targetOptions.map(language => ({ value: language, label: targetLanguageOptionLabel(settings, language) }))}
                disabled={model.targetOptions.length === 0 || (model.targetOptions.length === 1 && model.targetOptions[0] === settings.targetLanguage)}
                busy={sessionControlsBusy}
                onChange={value => performAction("target", () => onSwitchTargetLanguage(value as TargetLanguage), false)}
              />
            </div>
          </div>
          <div className="overlay-control-picker overlay-control-picker--profile" aria-busy={pendingAction === "profile"}>
            <Select label={I18N.settings.currentProfile} value={settings.activeProfileId ?? ""}
              valueLabel={I18N.settings.noActiveProfile}
              options={settings.profiles.map((profile) => ({ value: profile.id, label: profile.name, ...profileSelectionState(profile, settings.targetLanguage),
                icon: <ProviderIcon provider={profile.provider === "deepLX" ? "alibabaCloud" : profile.provider} size={32} /> }))}
              busy={sessionControlsBusy}
              onChange={(profileId) => {
                const profile = settings.profiles.find(profile => profile.id === profileId);
                if (!canChangeSessionSettings || !profile || profileSelectionState(profile, settings.targetLanguage).disabled) return;
                if (profileId !== settings.activeProfileId) {
                  performAction("profile", () => onSelectProfile(profileId), false);
                }
              }} />
            <SettingsHelp text={I18N.settings.profileSwitchHelp} label={`${I18N.settings.currentProfile} · ${I18N.settings.helpLabel}`} />
          </div>

          <OverlaySessionDetails
            onOpenProfile={() => performAction("settings", () => onShowSettings("activeProfile"))}
            disabled={pendingAction !== null}
          />

          {activeServiceProfile(settings)?.provider === "appleSpeech" && <div className="speech-resources-actions">
            <button type="button" className="speech-resources-link" disabled={pendingAction !== null}
              onClick={() => performAction("settings", () => onShowSettings("appleSpeechResources"))}>
              <Icon name="gear" />{I18N.settings.appleSpeechResources}
            </button>
          </div>}
          {sourceGuidance.notice && <div className="recognition-language-notice">{sourceGuidance.notice}</div>}
        </section>

        <CaptureStatusRow busy={pendingAction !== null} />

        <section className="overlay-control-section" aria-labelledby={subtitlesHeadingId}>
          <h2 className="overlay-control-section__title" id={subtitlesHeadingId}>{I18N.settings.subtitleTitle}</h2>
          <div
            ref={displayControlRef}
            className="overlay-control-display"
            role="group"
            aria-label={I18N.settings.subtitleDisplay}
            title={nativeShortcuts ? subtitleDisplayShortcut() : undefined}
          >
            {SUBTITLE_DISPLAY_OPTIONS.map(({ value, label }) => (
              <button
                key={value}
                type="button"
                aria-pressed={value === settings.subtitleDisplayMode}
                disabled={pendingAction !== null}
                onClick={() => {
                  if (value !== settings.subtitleDisplayMode) {
                    performAction("display", () => onSetSubtitleDisplayMode(value), false);
                  }
                }}
              >
                {label}
              </button>
            ))}
          </div>
        </section>

        <div className="overlay-control-footer">
          <button
            type="button"
            className="overlay-control-settings-link"
            disabled={pendingAction !== null}
            onClick={() => performAction("settings", onShowSettings, false)}
          >
            <span>{I18N.overlay.moreSettings}</span>
            <Icon name="chevron-right" />
          </button>
        </div>

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
