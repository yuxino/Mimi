import { useCallback, useEffect, useRef, useState } from "react";
import { SubtitleSessionControls } from "./SubtitleSessionControls";
import { Icon } from "../../components/Icon";
import { Switch } from "../../components/Switch";
import { I18N, setStoredUiLanguage, type UiLanguage } from "../../lib/i18n";
import { announceSettingsNavigationReady, isTauri, listenSettingsNavigation } from "../../lib/ipc";
import { selectSessionStatusKind, useStore } from "../../lib/store";
import type { SubtitleAlignment } from "../../lib/types";
import { SUBTITLE_DISPLAY_OPTIONS, subtitleDisplayShortcut } from "../../lib/subtitleDisplay";
import { subtitleBackgroundColor, subtitleColorHex } from "../../lib/subtitleColor";
import type { SubtitleDisplayMode } from "../../lib/types";
import { SubtitleColorControl } from "./SubtitleColorControl";
import { ServiceProfiles } from "./ServiceProfiles";
import { SupportDiagnostics } from "./SupportDiagnostics";
import { SessionExport } from "./SessionExport";
import { SoftwareUpdate } from "./SoftwareUpdate";
import { useSettingsTheme } from "./useSettingsTheme";
import { DockPreference } from "./DockPreference";
import { AppearancePicker } from "./AppearancePicker";
import { PulseRing } from "../overlay/PulseRing";
import type { PulseStyle } from "../../lib/types";
import { useResolvedMotion } from "../overlay/animation";
import { SettingsRow, SettingsSection, SettingsSelect } from "./SettingsPrimitives";
import { QuickStartGuide } from "./QuickStartGuide";
import { useDesktopShortcuts } from "../../lib/useDesktopShortcuts";
import { SettingsQuitFooter } from "./SettingsQuitFooter";
import { SettingsHelp } from "./SettingsHelp";
import { SettingsConfirmation } from "./DestructiveConfirmation";
import { SettingsInitializationStatus } from "./SettingsInitializationStatus";
import { AudioInputSettings } from "./AudioInputSettings";
import "./settings.css";

type SettingsCategory = "subtitles" | "service" | "general" | "export" | "diagnostics" | "guide";

const CATEGORY_SECTION_IDS: Record<SettingsCategory, string> = {
  subtitles: "subtitle-settings",
  service: "service-profiles",
  general: "application-settings",
  export: "session-export",
  diagnostics: "diagnostics",
  guide: "getting-started",
};

/** Compact settings surface shared by the macOS and Windows shells. */
export function SettingsView() {
  const { nativeShortcuts, commands: desktopShortcutCommands } = useDesktopShortcuts();
  const [showShortcutSetup, setShowShortcutSetup] = useState(false);
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

  const activeProfile =
    settings.profiles.find((profile) => profile.id === settings.activeProfileId) ??
    settings.profiles[0];
  const locationCategory = settingsCategoryFromHash(window.location.hash);
  const preferredCategory: SettingsCategory =
    activeProfile?.credentialState === "present" ? "subtitles" : "service";
  const [activeCategory, setActiveCategory] = useState<SettingsCategory>(
    locationCategory ?? preferredCategory,
  );
  const locationSelectedCategory = useRef(locationCategory !== null);
  const contentScrollRef = useRef<HTMLDivElement>(null);
  const initialCredentialState = useRef(activeProfile?.credentialState);

  // Native settings arrive after the first render. Resolve the initial
  // fail-closed `unavailable` placeholder once, without later pulling users
  // away from a category they chose or changing category after key edits.
  useEffect(() => {
    if (
      locationSelectedCategory.current ||
      initialCredentialState.current !== "unavailable" ||
      activeProfile?.credentialState === "unavailable"
    ) {
      return;
    }
    initialCredentialState.current = activeProfile?.credentialState;
    setActiveCategory(activeProfile?.credentialState === "present" ? "subtitles" : "service");
  }, [activeProfile?.credentialState]);

  const categories: readonly {
    id: SettingsCategory;
    label: string;
    icon: "captions-bubble" | "languages" | "gear" | "download" | "shield-check";
  }[] = [
    {
      id: "subtitles",
      label: I18N.settings.subtitleTitle,
      icon: "captions-bubble",
    },
    {
      id: "service",
      label: I18N.settings.serviceProfilesTitle,
      icon: "languages",
    },
    { id: "export", label: I18N.settings.sessionExportTitle, icon: "download" },
    {
      id: "general",
      label: I18N.settings.applicationTitle,
      icon: "gear",
    },
    { id: "diagnostics", label: I18N.settings.diagnosticsTitle, icon: "shield-check" },
  ];

  const pageDescriptions: Record<SettingsCategory, string> = {
    subtitles: I18N.settings.subtitlePageDescription,
    service: I18N.settings.servicePageDescription,
    general: I18N.settings.generalPageDescription,
    export: I18N.settings.exportPageDescription,
    diagnostics: I18N.settings.diagnosticsPageDescription,
    guide: I18N.settings.quickStartDescription,
  };

  const selectCategory = useCallback((category: SettingsCategory) => {
    locationSelectedCategory.current = true;
    setActiveCategory(category);
    contentScrollRef.current?.scrollTo({ top: 0 });
    window.history.replaceState(null, "", `#${CATEGORY_SECTION_IDS[category]}`);
  }, []);

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

    void listenSettingsNavigation((target) => {
      const category = target === "export" ? "export" : "service";
      selectCategory(category);
      window.requestAnimationFrame(() => {
        document.getElementById(`settings-category-${category}`)?.focus();
      });
    })
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
  }, [selectCategory]);

  return (
    <main className={`settings-console settings-console--${resolvedTheme}`}>
      <aside className="settings-sidebar">
        <div className="settings-sidebar-scroll">
          <div className="settings-brand">
            <span className="settings-brand__name">mimi</span>
            <span className="settings-brand__label">{I18N.settings.windowTitle}</span>
          </div>
          <nav
            className="settings-category-nav"
            aria-label={I18N.settings.windowTitle}
          >
            {categories.map((category) => {
              const selected = activeCategory === category.id;
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
          <header className="settings-page-header settings-page-header--help">
            <h1>{activeCategory === "guide" ? I18N.settings.quickStartTitle : categories.find((category) => category.id === activeCategory)?.label}</h1>
            <SettingsHelp text={pageDescriptions[activeCategory]} label={I18N.settings.helpLabel} />
          </header>
          {initializationReady && <SubtitleSessionControls visible={activeCategory === "subtitles"} onConfigure={() => selectCategory("service")} />}
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
                      <small>{I18N.settings.previewSample}</small>
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
                        style={{ fontSize: settings.fontSize, color: subtitleColorHex(settings.subtitleColor),
                          background: settings.subtitleBlendsWithBackground ? "transparent" : subtitleBackgroundColor(settings.subtitleBackgroundOpacity) }}
                      >
                        {settings.subtitleDisplayMode !== "translation" && (
                          <span style={{ color: settings.subtitleDisplayMode === "original" ? "inherit" : "rgba(255,255,255,0.72)" }}>{I18N.settings.previewOriginal}</span>
                        )}
                        {settings.subtitleDisplayMode !== "original" && (
                          <strong>{I18N.settings.previewTranslation}</strong>
                        )}
                      </div>
                    </div>
                    <div className="subtitle-preview__controls">
                      <SettingsRow label={I18N.settings.subtitleDisplay} description={`${I18N.settings.subtitleDisplayHelp}${nativeShortcuts ? ` ${subtitleDisplayShortcut()}` : ""}`}>
                        <SettingsSelect
                          label={I18N.settings.subtitleDisplay}
                          value={settings.subtitleDisplayMode}
                          options={SUBTITLE_DISPLAY_OPTIONS}
                          onChange={(value) => void saveSettings({ subtitleDisplayMode: value as SubtitleDisplayMode })}
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
                          onChange={(value) => void saveSettings({ pulseStyle: value as PulseStyle })}
                        />
                      </SettingsRow>
                      <SettingsRow label={I18N.settings.subtitleDividers} description={I18N.settings.subtitleDividersHelp}>
                        <Switch
                          checked={settings.showSubtitleDividers}
                          aria-label={I18N.settings.subtitleDividers}
                          onChange={(showSubtitleDividers) => void saveSettings({ showSubtitleDividers })}
                        />
                      </SettingsRow>
                      <SettingsRow
                        label={I18N.settings.pulseAnimation}
                      >
                        <Switch
                          checked={pulseOn}
                          aria-label={I18N.settings.pulseAnimation}
                          onChange={(pulseAnimation) =>
                            void saveSettings({ pulseAnimation })
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
                            void saveSettings({ subtitleAnimation })
                          }
                        />
                      </SettingsRow>
                      <SettingsRow label={I18N.settings.systemSubtitleColor}>
                        <SubtitleColorControl label={I18N.settings.systemSubtitleColor}
                          value={settings.subtitleColor}
                          onChange={(subtitleColor) => void saveSettings({ subtitleColor })}
                        />
                      </SettingsRow>
                      <SettingsRow label={I18N.settings.microphoneSubtitleColor}>
                        <SubtitleColorControl label={I18N.settings.microphoneSubtitleColor}
                          value={settings.microphoneSubtitleColor ?? "yellow"}
                          onChange={(microphoneSubtitleColor) => void saveSettings({ microphoneSubtitleColor })}
                        />
                      </SettingsRow>
                      <SettingsRow label={I18N.settings.fontSize}>
                        <div className="font-size-control">
                          <span className="font-size-control__sample" aria-hidden="true">
                            A
                          </span>
                          <input
                            type="range"
                            min={14}
                            max={20}
                            step={1}
                            value={settings.fontSize}
                            aria-label={I18N.settings.fontSize}
                            onChange={(event) =>
                              void saveSettings({
                                fontSize: Number(event.target.value),
                              })
                            }
                          />
                          <output aria-live="polite">{Math.round(settings.fontSize)}</output>
                        </div>
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
                            onChange={(event) => void saveSettings({
                              subtitleBackgroundOpacity: 100 - Number(event.target.value),
                            })}
                          />
                          <output aria-live="polite">{100 - (settings.subtitleBackgroundOpacity ?? 80)}%</output>
                        </div>
                      </SettingsRow>

                      <SettingsRow label={I18N.settings.subtitleAlignment}>
                        <SubtitleAlignmentControl
                          value={settings.subtitleAlignment}
                          onChange={(subtitleAlignment) => void saveSettings({ subtitleAlignment })}
                        />
                      </SettingsRow>
                    </div>
                  </div>
                  <div className="settings-divider" />

                  <details className="subtitle-placement">
                    <summary>
                      {I18N.settings.lockPosition}
                      <Icon name="chevron-down" />
                    </summary>
                    <SettingsRow
                      label={I18N.settings.lockPosition}
                      description={I18N.settings.lockHelp}
                      align="start"
                    >
                      <Switch
                        checked={settings.isOverlayLocked}
                        aria-label={I18N.settings.lockPosition}
                        onChange={(checked) => {
                          void setOverlayLocked(checked).catch(() => {});
                        }}
                      />
                    </SettingsRow>
                  </details>
                </SettingsSection>
              </div>
            )}

            <div id="service-profiles-panel" className={`settings-category-panel${activeCategory !== "service" ? " is-inactive" : ""}`}>
                <AudioInputSettings />
                <ServiceProfiles settings={settings} sessionIsActive={sessionIsActive} sessionIsPaused={sessionIsPaused} sessionStatusKind={sessionStatusKind} visible={activeCategory === "service"} />
              </div>

            <div id="diagnostics-panel" className={`settings-category-panel${activeCategory !== "diagnostics" ? " is-inactive" : ""}`}>
              <SupportDiagnostics visible={activeCategory === "diagnostics"} />
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
                  description={I18N.settings.languageHelp}
                  align="start"
                >
                  <SettingsSelect
                    value={settings.uiLanguage ?? "system"}
                    label={I18N.settings.appLanguage}
                    onChange={(value) => {
                      const language = value as UiLanguage;
                      void saveSettings({ uiLanguage: language })
                        .then(() => {
                          setStoredUiLanguage(language);
                        })
                        .catch(() => {});
                    }}
                    options={[
                      {
                        value: "system",
                        label: I18N.settings.systemLanguage,
                      },
                      { value: "zh", label: I18N.settings.chinese },
                      { value: "en", label: I18N.settings.english },
                      { value: "ja", label: I18N.settings.japanese },
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
                      <dt>{I18N.settings.subtitleDisplay}</dt><dd><code>{desktopShortcutCommands.cycleSubtitleDisplay}</code></dd>
                    </dl>
                  </div>
                </SettingsConfirmation>}

                <div className="settings-divider" />

                <DockPreference />

                <SoftwareUpdate />
              </SettingsSection>
            </div>
            <div
              id="session-export-panel"
              className={`settings-category-panel${activeCategory !== "export" ? " is-inactive" : ""}`}
            >
              <SessionExport visible={activeCategory === "export"} />
            </div>
            </>}
          </div>
        </div>
      </div>
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
    case "network-proxy":
    case "translation-languages":
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
