import { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { SubtitleSessionControls } from "./session/SubtitleSessionControls";
import { Icon } from "../../components/Icon";
import { Switch } from "../../components/inputs/Switch";
import { UI_LANGUAGE_OPTIONS, I18N, setStoredUiLanguage, type UiLanguage } from "../../lib/i18n";
import { announceSettingsNavigationReady, isTauri, listenSettingsNavigation, type SettingsNavigationTarget } from "../../lib/ipc";
import { selectSessionStatusKind, useStore } from "../../lib/store";
import { credentialStateForTarget } from "../../lib/providers/providerCapabilities";
import type { SettingsDraft, SubtitleAlignment } from "../../lib/types";
import { SUBTITLE_DISPLAY_OPTIONS, VISIBLE_SUBTITLE_COUNT_OPTIONS, visibleSubtitleCountChoice, visibleSubtitleCountDraft, subtitleDisplayShortcut } from "../../lib/subtitles/subtitleDisplay";
import {
  isLightSubtitleBackground,
  SUBTITLE_BACKGROUND_COLOR_OPTIONS,
  subtitleBackgroundColor,
  subtitleBackgroundHex,
} from "../../lib/subtitles/subtitleColor";
import type { SubtitleDisplayMode } from "../../lib/types";
import { MAX_SUBTITLE_FONT_SIZE, MIN_SUBTITLE_FONT_SIZE, subtitleTextStyle } from "../../lib/subtitles/subtitleTypography";
import { subtitleFontFamily } from "../../lib/subtitles/subtitleFont";
import { SubtitleFontControl } from "./appearance/SubtitleFontControl";
import { SubtitleColorControl } from "./appearance/SubtitleColorControl";
import { ServiceProfiles } from "./services/ServiceProfiles";
import { SupportDiagnostics } from "./support/SupportDiagnostics";
import { SessionExport } from "./history/SessionExport";
import { SoftwareUpdate } from "./updates/SoftwareUpdate";
import { useSettingsTheme } from "./appearance/useSettingsTheme";
import { DockPreference } from "./appearance/DockPreference";
import { AppearancePicker } from "./appearance/AppearancePicker";
import { PulseRing } from "../overlay/status/PulseRing";
import type { PulseStyle } from "../../lib/types";
import { useResolvedMotion } from "../overlay/timeline/animation";
import { SettingsRow, SettingsSection, SettingsSelect } from "./controls/SettingsPrimitives";
import { QuickStartGuide } from "./guide/QuickStartGuide";
import { useDesktopShortcuts } from "../../lib/shortcuts/useDesktopShortcuts";
import { SettingsQuitFooter } from "./navigation/SettingsQuitFooter";
import { SettingsHelp } from "./feedback/SettingsHelp";
import { SettingsConfirmation } from "./controls/SettingsConfirmation";
import { SettingsInitializationStatus } from "./feedback/SettingsInitializationStatus";
import { VoiceSettings } from "./audio/VoiceSettings";
import { settingsPageCopy } from "./navigation/SettingsPageCopy";
import { SettingsToastRegion } from "./feedback/SettingsToast";
import { useSettingsToast } from "./feedback/useSettingsToast";
import "./settings.css";

const DevelopmentDebugger = __MIMI_DEVELOPMENT_BUILD__
  ? lazy(() => import("./development/DevelopmentDebugger").then(module => ({ default: module.DevelopmentDebugger })))
  : null;

type SettingsCategory = "subtitles" | "voice" | "service" | "general" | "export" | "diagnostics" | "guide";

const CATEGORY_SECTION_IDS: Record<SettingsCategory, string> = {
  subtitles: "subtitle-settings",
  service: "service-profiles",
  voice: "voice-settings",
  general: "application-settings",
  export: "session-export",
  diagnostics: "diagnostics",
  guide: "getting-started",
};

/** Compact settings surface shared by the macOS and Windows shells. */
export function SettingsView() {
  const { nativeShortcuts, commands: desktopShortcutCommands } = useDesktopShortcuts();
  const isMac = /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent);
  const [showShortcutSetup, setShowShortcutSetup] = useState(false);
  const [subtitleFontPreview, setSubtitleFontPreview] = useState<string | undefined>();
  const { theme, resolvedTheme, changeTheme } = useSettingsTheme();
  // Subscribe only to state rendered in this window. Subtitle text updates do
  // not re-render settings while a stream is active.
  const sessionStatusKind = useStore(selectSessionStatusKind);
  const sessionIsActive = useStore((state) => state.session.isActive);
  const sessionIsPaused = useStore((state) => state.session.isPaused);
  const settings = useStore((state) => state.settings);
  const initializationStatus = useStore((state) => state.initializationStatus);
  const initializationError = useStore((state) => state.initializationError);
  const initialize = useStore((state) => state.init);
  const initializationReady = initializationStatus === "ready";
  // Show the actual on/off state, including compatible legacy preferences.
  // User changes persist an explicit boolean through saveSettings.
  const pulseOn = useResolvedMotion(settings.pulseAnimation);
  const motionOn = useResolvedMotion(settings.subtitleAnimation);
  const saveSettings = useStore((state) => state.saveSettings);
  const setOverlayLocked = useStore((state) => state.setOverlayLocked);
  const quit = useStore((state) => state.quit);
  const { runWithToast } = useSettingsToast();
  const savePreference = (draft: SettingsDraft, label: string) =>
    runWithToast(() => saveSettings(draft), I18N.settings.settingSaveFailed(label));
  const previewFontFamily = useMemo(() => subtitleFontFamily(subtitleFontPreview ?? settings.subtitleFontFamily),
    [subtitleFontPreview, settings.subtitleFontFamily]);
  const previewLightSurface = !settings.subtitleBlendsWithBackground
    && isLightSubtitleBackground(settings.subtitleBackgroundColor, settings.subtitleBackgroundOpacity);
  const previewTextStyle = (kind: "source" | "translation") => subtitleTextStyle({
    fontSize: settings.fontSize,
    fontFamily: previewFontFamily,
    displayMode: settings.subtitleDisplayMode,
    kind,
    color: settings.audioInput === "microphone" ? settings.microphoneSubtitleColor ?? "yellow" : settings.subtitleColor,
    blendsWithBackground: settings.subtitleBlendsWithBackground,
    lightBackground: previewLightSurface,
    lowOpacityBackground: !settings.subtitleBlendsWithBackground && (settings.subtitleBackgroundOpacity ?? 80) < 35,
  });

  const activeProfile =
    settings.profiles.find((profile) => profile.id === settings.activeProfileId) ??
    settings.profiles[0];
  const locationCategory = settingsCategoryFromHash(window.location.hash);
  const preferredCategory: SettingsCategory =
    credentialStateForTarget(activeProfile, settings.targetLanguage) === "present" ? "subtitles" : "service";
  const [activeCategory, setActiveCategory] = useState<SettingsCategory>(
    locationCategory ?? preferredCategory,
  );
  const [initialCategoryResolved, setInitialCategoryResolved] = useState(initializationReady);
  const contentScrollRef = useRef<HTMLDivElement>(null);
  const [appleResourcesRequest, setAppleResourcesRequest] = useState(0);
  const appleResourcesSequence = useRef(0);
  const [profileEditorRequest, setProfileEditorRequest] = useState(0);
  const [sessionReaderOpen, setSessionReaderOpen] = useState(false);
  const profileEditorSequence = useRef(0);
  const diagnosticNavigationFocus = useRef<string | null>(null);

  // Resolve the first native snapshot once. An empty catalog opens services;
  // later additions and credential changes preserve the user's current page.
  if (!initialCategoryResolved && initializationReady) {
    setInitialCategoryResolved(true);
    setActiveCategory(locationCategory ?? preferredCategory);
  }

  const categories: readonly {
    id: SettingsCategory;
    label: string;
    icon: "captions-bubble" | "languages" | "layers" | "gear" | "download";
  }[] = [
    {
      id: "subtitles",
      label: I18N.settings.subtitleTitle,
      icon: "captions-bubble",
    },
    { id: "voice", label: settingsPageCopy().voice, icon: "languages" },
    { id: "service", label: settingsPageCopy().models, icon: "layers" },
    { id: "export", label: I18N.settings.sessionExportTitle, icon: "download" },
    {
      id: "general",
      label: I18N.settings.applicationTitle,
      icon: "gear",
    },
  ];

  const pageDescriptions: Partial<Record<SettingsCategory, string>> = {
    export: I18N.settings.exportPageDescription,
    diagnostics: I18N.settings.diagnosticsHelp,
  };

  const selectCategory = useCallback((category: SettingsCategory) => {
    setAppleResourcesRequest(0);
    setProfileEditorRequest(0);
    setActiveCategory(category);
    contentScrollRef.current?.scrollTo({ top: 0 });
    window.history.replaceState(null, "", `#${CATEGORY_SECTION_IDS[category]}`);
  }, []);

  useLayoutEffect(() => {
    if (contentScrollRef.current) contentScrollRef.current.scrollTop = 0;
  }, [activeCategory]);

  useEffect(() => {
    if (diagnosticNavigationFocus.current === null) return;
    document.getElementById(diagnosticNavigationFocus.current)?.focus();
    diagnosticNavigationFocus.current = null;
  }, [activeCategory]);

  const navigateToSettings = useCallback((target: SettingsNavigationTarget = "service") => {
    const category = target === "export" ? "export" : target === "voice" ? "voice" : "service";
    selectCategory(category);
    if (target === "activeProfile") {
      profileEditorSequence.current += 1;
      setProfileEditorRequest(profileEditorSequence.current);
    } else if (target === "appleSpeechResources") {
      appleResourcesSequence.current += 1;
      setAppleResourcesRequest(appleResourcesSequence.current);
    } else {
      window.requestAnimationFrame(() => {
        document.getElementById(`settings-category-${category}`)?.focus();
      });
    }
  }, [selectCategory]);

  useEffect(() => {
    const navigateFromHash = () => {
      const category = settingsCategoryFromHash(window.location.hash);
      if (category) selectCategory(category);
    };
    window.addEventListener("hashchange", navigateFromHash);
    return () => window.removeEventListener("hashchange", navigateFromHash);
  }, [selectCategory]);

  useEffect(() => {
    if (!isTauri) return;

    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listenSettingsNavigation(navigateToSettings)
      .then(async (installedUnlisten) => {
        if (disposed) {
          installedUnlisten();
          return;
        }
        unlisten = installedUnlisten;
        await announceSettingsNavigationReady();
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [navigateToSettings]);

  return (
    <main className={`settings-console settings-console--${resolvedTheme}`}>
      <aside className="settings-sidebar">
        <div className="settings-sidebar-scroll">
          <div className="settings-brand">
            <span className="settings-brand__name">Mimi</span>
            <span className="settings-brand__label">{I18N.settings.windowTitle}</span>
          </div>
          <nav
            className="settings-category-nav"
            aria-label={I18N.settings.windowTitle}
          >
            {categories.map((category) => {
              const selected = activeCategory === category.id || (category.id === "general" && activeCategory === "diagnostics");
              return (
                <button
                  key={category.id}
                  id={`settings-category-${category.id}`}
                  type="button"
                  className={`settings-category-nav__item${selected ? " is-selected" : ""}`}
                  aria-current={selected ? "page" : undefined}
                  aria-controls={`${CATEGORY_SECTION_IDS[category.id]}-panel`}
                  onClick={() => selectCategory(category.id)}
                >
                  <Icon name={category.icon} />
                  <span>{category.label}</span>
                </button>
              );
            })}
          </nav>
          <div className="settings-sidebar-support">
            <button
              type="button"
              className={`settings-category-nav__item settings-guide-entry${activeCategory === "guide" ? " is-selected" : ""}`}
              aria-current={activeCategory === "guide" ? "page" : undefined}
              aria-controls="getting-started-panel"
              onClick={() => selectCategory("guide")}
            >
              <Icon name="captions-bubble" />
              <span>{I18N.settings.quickStartNav}</span>
            </button>
          </div>
        </div>
        <SettingsQuitFooter onQuit={quit} />
      </aside>
      <div className="settings-console__scroll" ref={contentScrollRef}>
        <div className="settings-console__frame">
          {!(activeCategory === "export" && sessionReaderOpen) && <header className="settings-page-header settings-page-header--help">
            <h1>{activeCategory === "guide" ? I18N.settings.quickStartTitle : activeCategory === "diagnostics" ? I18N.settings.diagnosticsTitle : categories.find((category) => category.id === activeCategory)?.label}</h1>
            {pageDescriptions[activeCategory] && <SettingsHelp text={pageDescriptions[activeCategory]} label={I18N.settings.helpLabel} />}
          </header>}
          {initializationReady && <SubtitleSessionControls visible={activeCategory === "subtitles"} onConfigure={navigateToSettings} />}
          <div className="settings-layout">
            {!initializationReady ? <SettingsInitializationStatus status={initializationStatus} error={initializationError} onRetry={() => { void initialize(); }} /> : <>
            {activeCategory === "guide" && (
              <div id="getting-started-panel" className="settings-category-panel">
                <QuickStartGuide
                  onConfigureService={() => selectCategory("service")}
                  onOpenSubtitles={() => selectCategory("subtitles")}
                />
              </div>
            )}
            {activeCategory === "subtitles" && (
              <div id="subtitle-settings-panel" className="settings-category-panel">
                <SettingsSection
                  id="subtitle-settings"
                  title={I18N.settings.subtitleTitle}
                  hideHeading
                >
                  <div
                    className="subtitle-preview"
                    data-immersive={settings.subtitleBlendsWithBackground}
                  >
                    <div className="subtitle-preview__label">
                      <span>{I18N.settings.subtitlePreview}</span>
                    </div>
                    <div
                      className="subtitle-preview__stage"
                      style={{ textAlign: settings.subtitleAlignment }}
                    >
                      <div className="subtitle-preview__pulse">
                        <PulseRing phase="listening" pulseStyle={settings.pulseStyle} motionEnabled={pulseOn} />
                      </div>
                      <div
                        className="subtitle-preview__text"
                        data-light-surface={previewLightSurface ? "" : undefined}
                        style={{
                          background: settings.subtitleBlendsWithBackground
                            ? "transparent"
                            : subtitleBackgroundColor(settings.subtitleBackgroundOpacity, settings.subtitleBackgroundColor),
                        }}
                      >
                        {settings.subtitleDisplayMode !== "translation" && (
                          <span style={previewTextStyle("source")}>{I18N.settings.previewOriginal}</span>
                        )}
                        {settings.subtitleDisplayMode !== "original" && (
                          <strong style={previewTextStyle("translation")}>{I18N.settings.previewTranslation}</strong>
                        )}
                      </div>
                    </div>
                    <div className="subtitle-preview__controls">
                      <div className="subtitle-preview__selectors">
                        <SettingsRow label={I18N.settings.subtitleDisplay} description={I18N.settings.subtitleDisplayHelp} shortcut={nativeShortcuts ? subtitleDisplayShortcut() : undefined}>
                          <SettingsSelect
                            label={I18N.settings.subtitleDisplay}
                            value={settings.subtitleDisplayMode}
                            options={SUBTITLE_DISPLAY_OPTIONS}
                            onChange={(value) => void savePreference({ subtitleDisplayMode: value as SubtitleDisplayMode }, I18N.settings.subtitleDisplay)}
                          />
                        </SettingsRow>
                        <SubtitleFontControl value={settings.subtitleFontFamily ?? ""}
                          onPreview={setSubtitleFontPreview}
                          getCurrentValue={() => useStore.getState().settings.subtitleFontFamily ?? ""}
                          onChange={subtitleFontFamily => saveSettings({ subtitleFontFamily })} />
                      </div>
                      <SettingsRow label={I18N.settings.showIntermediateSubtitles} description={I18N.settings.showIntermediateSubtitlesHelp}>
                        <Switch
                          checked={settings.showIntermediateSubtitles !== false}
                          aria-label={I18N.settings.showIntermediateSubtitles}
                          onChange={(showIntermediateSubtitles) => void savePreference({ showIntermediateSubtitles }, I18N.settings.showIntermediateSubtitles)}
                        />
                      </SettingsRow>
                      <SettingsRow label={I18N.settings.pulseStyle}>
                        <SettingsSelect
                          label={I18N.settings.pulseStyle}
                          value={settings.pulseStyle}
                          options={[
                            { value: "syllable", label: I18N.settings.pulseStyleSyllable },
                            { value: "ribbon", label: I18N.settings.pulseStyleRibbon },
                          ]}
                          onChange={(value) => void savePreference({ pulseStyle: value as PulseStyle }, I18N.settings.pulseStyle)}
                        />
                      </SettingsRow>
                      <SettingsRow label={I18N.settings.subtitleDividers} description={I18N.settings.subtitleDividersHelp}>
                        <Switch
                          checked={settings.showSubtitleDividers}
                          aria-label={I18N.settings.subtitleDividers}
                          onChange={(showSubtitleDividers) => void savePreference({ showSubtitleDividers }, I18N.settings.subtitleDividers)}
                        />
                      </SettingsRow>
                      <SettingsRow label={I18N.settings.subtitleTimestamps} description={I18N.settings.subtitleTimestampsHelp}>
                        <Switch
                          checked={settings.showSubtitleTimestamps ?? false}
                          aria-label={I18N.settings.subtitleTimestamps}
                          onChange={(showSubtitleTimestamps) => void savePreference({ showSubtitleTimestamps }, I18N.settings.subtitleTimestamps)}
                        />
                      </SettingsRow>
                      <SettingsRow label={I18N.settings.visibleSubtitleCount} description={I18N.settings.visibleSubtitleCountHelp}>
                        <SettingsSelect
                          label={I18N.settings.visibleSubtitleCount}
                          value={visibleSubtitleCountChoice(settings)}
                          options={VISIBLE_SUBTITLE_COUNT_OPTIONS}
                          onChange={(value) => void savePreference(visibleSubtitleCountDraft(value), I18N.settings.visibleSubtitleCount)}
                        />
                      </SettingsRow>
                      <SettingsRow
                        label={I18N.settings.pulseAnimation}
                      >
                        <Switch
                          checked={pulseOn}
                          aria-label={I18N.settings.pulseAnimation}
                          onChange={(pulseAnimation) =>
                            void savePreference({ pulseAnimation }, I18N.settings.pulseAnimation)
                          }
                        />
                      </SettingsRow>
                      <SettingsRow
                        label={I18N.settings.textAnimation}
                        description={I18N.settings.textAnimationHelp}
                        align="start"
                      >
                        <Switch
                          checked={motionOn}
                          aria-label={I18N.settings.textAnimation}
                          onChange={(subtitleAnimation) =>
                            void savePreference({ subtitleAnimation }, I18N.settings.textAnimation)
                          }
                        />
                      </SettingsRow>
                      <SettingsRow label={I18N.settings.systemSubtitleColor}>
                        <SubtitleColorControl label={I18N.settings.systemSubtitleColor}
                          value={settings.subtitleColor}
                          onChange={(subtitleColor) => void savePreference({ subtitleColor }, I18N.settings.systemSubtitleColor)}
                        />
                      </SettingsRow>
                      {settings.microphoneInputAvailable && <SettingsRow label={I18N.settings.microphoneSubtitleColor}>
                        <SubtitleColorControl label={I18N.settings.microphoneSubtitleColor}
                          value={settings.microphoneSubtitleColor ?? "yellow"}
                          onChange={(microphoneSubtitleColor) => void savePreference({ microphoneSubtitleColor }, I18N.settings.microphoneSubtitleColor)}
                        />
                      </SettingsRow>}
                      <SettingsRow label={I18N.settings.fontSize}>
                        <div className="font-size-control">
                          <span className="font-size-control__sample" aria-hidden="true">
                            A
                          </span>
                          <input
                            type="range"
                            min={MIN_SUBTITLE_FONT_SIZE}
                            max={MAX_SUBTITLE_FONT_SIZE}
                            step={1}
                            value={settings.fontSize}
                            aria-label={I18N.settings.fontSize}
                            onChange={(event) =>
                              void savePreference({
                                fontSize: Number(event.target.value),
                              }, I18N.settings.fontSize)
                            }
                          />
                          <output aria-live="polite">{Math.round(settings.fontSize)}</output>
                        </div>
                      </SettingsRow>

                      <SettingsRow label={I18N.settings.backgroundColor}>
                        <SubtitleColorControl
                          label={I18N.settings.backgroundColor}
                          value={subtitleBackgroundHex(settings.subtitleBackgroundColor)}
                          options={SUBTITLE_BACKGROUND_COLOR_OPTIONS.map((option) => ({
                            value: option.value,
                            hex: option.value,
                            get label() { return option.label; },
                          }))}
                          resolveHex={subtitleBackgroundHex}
                          disabled={settings.subtitleBlendsWithBackground}
                          onChange={(subtitleBackgroundColor) => void savePreference(
                            { subtitleBackgroundColor },
                            I18N.settings.backgroundColor,
                          )}
                        />
                      </SettingsRow>

                      <SettingsRow label={I18N.settings.backgroundTransparency}>
                        <div className="font-size-control background-transparency-control">
                          <input
                            type="range"
                            min={0}
                            max={100}
                            step={1}
                            value={100 - (settings.subtitleBackgroundOpacity ?? 80)}
                            aria-label={I18N.settings.backgroundTransparency}
                            disabled={settings.subtitleBlendsWithBackground}
                            onChange={(event) => void savePreference({
                              subtitleBackgroundOpacity: 100 - Number(event.target.value),
                            }, I18N.settings.backgroundTransparency)}
                          />
                          <output aria-live="polite">{100 - (settings.subtitleBackgroundOpacity ?? 80)}%</output>
                        </div>
                      </SettingsRow>

                      <SettingsRow label={I18N.settings.subtitleAlignment}>
                        <SubtitleAlignmentControl
                          value={settings.subtitleAlignment}
                          onChange={(subtitleAlignment) => void savePreference({ subtitleAlignment }, I18N.settings.subtitleAlignment)}
                        />
                      </SettingsRow>
                      <SettingsRow
                        label={I18N.settings.lockPosition}
                        description={I18N.settings.lockHelp}
                        descriptionId="subtitle-position-lock-help"
                        shortcut={nativeShortcuts ? isMac ? "⌘⇧L" : "Ctrl+Shift+L" : undefined}
                        shortcutId="subtitle-position-lock-shortcut"
                      >
                        <Switch
                          checked={settings.isOverlayLocked}
                          aria-label={I18N.settings.lockPosition}
                          aria-describedby={nativeShortcuts ? "subtitle-position-lock-help subtitle-position-lock-shortcut" : "subtitle-position-lock-help"}
                          onChange={(checked) => {
                            void runWithToast(() => setOverlayLocked(checked), I18N.settings.settingSaveFailed(I18N.settings.lockPosition));
                          }}
                        />
                      </SettingsRow>
                    </div>
                  </div>
                </SettingsSection>
              </div>
            )}

            {activeCategory === "voice" && <div id="voice-settings-panel" className="settings-category-panel">
              <VoiceSettings onOpenModels={() => selectCategory("service")} onOpenAppleResources={() => navigateToSettings("appleSpeechResources")} />
            </div>}
            <div id="service-profiles-panel" className={`settings-category-panel${activeCategory !== "service" ? " is-inactive" : ""}`}>
                <ServiceProfiles settings={settings} appleResourcesRequest={appleResourcesRequest} profileEditorRequest={profileEditorRequest} sessionIsActive={sessionIsActive} sessionIsPaused={sessionIsPaused} sessionStatusKind={sessionStatusKind} visible={activeCategory === "service"} />
              </div>

            <div id="diagnostics-panel" className={`settings-category-panel${activeCategory !== "diagnostics" ? " is-inactive" : ""}`}>
              <button id="settings-diagnostics-back" type="button" className="settings-button settings-button--quiet service-back settings-diagnostics-back" onClick={() => {
                diagnosticNavigationFocus.current = "settings-category-diagnostics";
                selectCategory("general");
              }}>
                <Icon name="chevron-left" />{I18N.settings.backToGeneral}
              </button>
              <SupportDiagnostics visible={activeCategory === "diagnostics"}
                onOpenAudio={() => navigateToSettings("voice")}
                onOpenServices={() => navigateToSettings("service")} />
              {activeCategory === "diagnostics" && DevelopmentDebugger !== null && (
                <Suspense fallback={null}><DevelopmentDebugger visible /></Suspense>
              )}
            </div>

            <div
              id="application-settings-panel"
              className={`settings-category-panel${activeCategory !== "general" ? " is-inactive" : ""}`}
            >
              <SettingsSection id="application-settings" title={I18N.settings.appearance}>
                <AppearancePicker value={theme} onChange={changeTheme} />
              </SettingsSection>
              <SettingsSection id="application-preferences" title={I18N.settings.preferencesTitle}>
                <SettingsRow
                  label={I18N.settings.appLanguage}
                  align="start"
                >
                  <SettingsSelect
                    value={settings.uiLanguage ?? "system"}
                    label={I18N.settings.appLanguage}
                    onChange={(value) => {
                      const language = value as UiLanguage;
                      void runWithToast(async () => {
                        await saveSettings({ uiLanguage: language });
                        setStoredUiLanguage(language);
                      }, I18N.settings.languageSaveFailed);
                    }}
                    options={[
                      {
                        value: "system",
                        label: I18N.settings.systemLanguage,
                      },
                      ...UI_LANGUAGE_OPTIONS,
                    ]}
                  />
                </SettingsRow>

                {desktopShortcutCommands && <div className="settings-shortcut-setup">
                  <button type="button" className="settings-button settings-button--quiet" onClick={() => setShowShortcutSetup(true)}>
                    <Icon name="gear" />{I18N.settings.systemShortcutSetup}
                  </button>
                </div>}
                {desktopShortcutCommands && showShortcutSetup && activeCategory === "general" && <SettingsConfirmation
                  message={I18N.settings.systemShortcutSetup} variant="default" hideConfirm cancelLabel={I18N.settings.closeDialog}
                  onCancel={() => setShowShortcutSetup(false)} onConfirm={() => setShowShortcutSetup(false)}>
                  <div className="settings-desktop-shortcuts">
                    <p>{I18N.settings.systemShortcutInstructions}</p>
                    <dl>
                      <dt>{I18N.settings.startStopShortcut}</dt><dd><code>{desktopShortcutCommands.toggleSession}</code></dd>
                      <dt>{I18N.tray.blendBackground}</dt><dd><code>{desktopShortcutCommands.toggleImmersive}</code></dd>
                      <dt>{I18N.settings.lockPosition}</dt><dd><code>{desktopShortcutCommands.toggleLock}</code></dd>
                      <dt>{I18N.settings.subtitleDisplay}</dt><dd><code>{desktopShortcutCommands.cycleSubtitleDisplay}</code></dd>
                    </dl>
                  </div>
                </SettingsConfirmation>}

                <div className="settings-divider" />

                <DockPreference />

                <SoftwareUpdate active={activeCategory === "general"} />
              </SettingsSection>
              <SettingsSection id="advanced-settings" title={I18N.settings.advancedTitle}>
                <SettingsRow label={I18N.settings.diagnosticsTitle}>
                  <button id="settings-category-diagnostics" type="button" className="settings-button settings-button--quiet" aria-controls="diagnostics-panel" onClick={() => {
                    diagnosticNavigationFocus.current = "settings-diagnostics-back";
                    selectCategory("diagnostics");
                  }}>
                    {I18N.settings.openDiagnostics}<Icon name="chevron-right" />
                  </button>
                </SettingsRow>
              </SettingsSection>
            </div>
            <div
              id="session-export-panel"
              className={`settings-category-panel${activeCategory !== "export" ? " is-inactive" : ""}`}
            >
              <SessionExport visible={activeCategory === "export"} onReaderChange={setSessionReaderOpen} />
            </div>
            </>}
          </div>
        </div>
      </div>
      <SettingsToastRegion scopeKey={activeCategory} />
    </main>
  );
}

const SUBTITLE_ALIGNMENTS: readonly SubtitleAlignment[] = ["left", "center", "right"];

function SubtitleAlignmentControl({
  value,
  onChange,
}: {
  value: SubtitleAlignment;
  onChange: (alignment: SubtitleAlignment) => void;
}) {
  const labels: Record<SubtitleAlignment, string> = {
    left: I18N.settings.alignLeft,
    center: I18N.settings.alignCenter,
    right: I18N.settings.alignRight,
  };

  return (
    <div
      className="subtitle-alignment-control"
      role="group"
      aria-label={I18N.settings.subtitleAlignment}
    >
      {SUBTITLE_ALIGNMENTS.map((alignment) => (
        <button
          key={alignment}
          type="button"
          className={value === alignment ? "is-selected" : undefined}
          aria-label={labels[alignment]}
          aria-pressed={value === alignment}
          title={labels[alignment]}
          onClick={() => onChange(alignment)}
        >
          <Icon name={`align-${alignment}`} />
        </button>
      ))}
    </div>
  );
}

function settingsCategoryFromHash(hash: string): SettingsCategory | null {
  switch (hash.replace(/^#/, "")) {
    case CATEGORY_SECTION_IDS.subtitles:
      return "subtitles";
    case "translation-languages":
    case CATEGORY_SECTION_IDS.voice:
      return "voice";
    case "network-proxy":
    case CATEGORY_SECTION_IDS.service:
      return "service";
    case CATEGORY_SECTION_IDS.general:
      return "general";
    case CATEGORY_SECTION_IDS.export:
      return "export";
    case CATEGORY_SECTION_IDS.diagnostics:
      return "diagnostics";
    case CATEGORY_SECTION_IDS.guide:
      return "guide";
    default:
      return null;
  }
}
