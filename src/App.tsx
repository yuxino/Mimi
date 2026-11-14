import { getCurrentWindow } from "@tauri-apps/api/window";
import { lazy, Suspense, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type ComponentType } from "react";
import { effectiveUiLanguage, subscribeUiLanguage } from "./lib/i18n";
import { appUiTestFrontendReady, isTauri } from "./lib/ipc";
import { verifyUiTestConnectionReadiness } from "./lib/uiTestConnectionReadiness";
import { selectSessionStatusKind, useStore } from "./lib/store";
import { useDesktopShortcuts } from "./lib/useDesktopShortcuts";
import { loadSettingsView, settingsStartup } from "./settingsStartupProbe";

const OverlayWindow = lazy(() =>
  import("./windows/overlay/OverlayWindow").then((module) => ({
    default: module.OverlayWindow,
  })),
);
const OverlayControlWindow = lazy(() =>
  import("./windows/overlay-control/OverlayControlWindow").then((module) => ({
    default: module.OverlayControlWindow,
  })),
);
const SettingsView = lazy(loadSettingsView);
type WindowLabel = "overlay" | "overlay-control" | "settings";

/**
 * Every window loads the shared entry, then lazily loads only the component
 * matching its label: "overlay" (floating subtitles), "settings" (main settings
 * window), or
 * "overlay-control" (the child status island and control panel). Outside
 * Tauri a `?window=` query parameter selects the preview (defaults to
 * "settings").
 */
export default function App({ settingsView: PreloadedSettingsView }: { settingsView?: ComponentType }) {
  const uiLanguage = useSyncExternalStore(subscribeUiLanguage, effectiveUiLanguage);
  const [label] = useState<WindowLabel>(resolveInitialLabel);
  const init = useStore((state) => state.init);

  useLayoutEffect(() => {
    document.documentElement.lang = uiLanguage === "zh" ? "zh-CN" : uiLanguage;
  }, [uiLanguage]);

  useEffect(() => {
    if (label === "settings" && settingsStartup[1] === 0) {
      settingsStartup[1] = performance.now();
      void init().then(() => { settingsStartup[4] = performance.now(); });
    } else {
      void init();
    }
  }, [init, label]);

  useEffect(() => {
    document.body.classList.toggle("settings-body", label === "settings");
  }, [label]);

  let windowContent;
  if (label === "overlay") windowContent = <OverlayWindow />;
  else if (label === "overlay-control") windowContent = <OverlayControlWindow />;
  else windowContent = PreloadedSettingsView ? <PreloadedSettingsView /> : <SettingsView />;

  return (
    <Suspense fallback={null}>
      {windowContent}
      <FrontendReadySignal label={label} />
    </Suspense>
  );
}

/** Mount inside Suspense so a failed lazy import cannot pass native smoke. */
function FrontendReadySignal({ label }: { label: WindowLabel }) {
  const { commands: desktopShortcuts } = useDesktopShortcuts();
  const status = useStore(selectSessionStatusKind);
  const settingsInitialized = useStore((state) => state.initialized);
  const reported = useRef(false);

  useLayoutEffect(() => {
    if (label === "settings" && settingsInitialized && settingsStartup[5] === 0) {
      settingsStartup[5] = performance.now();
    }
  }, [label, settingsInitialized]);

  useEffect(() => {
    if (
      !isTauri ||
      reported.current ||
      desktopShortcuts === undefined ||
      status !== "listening" ||
      (label !== "settings" && label !== "overlay")
    ) {
      return;
    }

    // Wait for the committed window to paint with the native session snapshot.
    // Shortcut hydration must succeed too, so missing IPC capabilities fail
    // installed-app smoke instead of silently hiding all shortcut hints.
    // The backend writes a content-free marker only in explicit UI-test mode.
    let secondFrame = 0;
    const firstFrame = window.requestAnimationFrame(() => {
      secondFrame = window.requestAnimationFrame(() => {
        // Native package smoke exercises the real IPC registration, window
        // capability and response shape. Production never probes a provider here.
        const diagnostics = label === "settings"
          ? verifyUiTestConnectionReadiness(useStore.getState().settings.activeProfileId)
          : Promise.resolve();
        void diagnostics.then(() => appUiTestFrontendReady(label === "settings" ? settingsStartup : undefined))
          .then(() => {
            reported.current = true;
          })
          .catch(() => {});
      });
    });
    return () => {
      window.cancelAnimationFrame(firstFrame);
      window.cancelAnimationFrame(secondFrame);
    };
  }, [label, status, desktopShortcuts]);

  return null;
}

function resolveInitialLabel(): WindowLabel {
  if (isTauri) {
    return getCurrentWindow().label as WindowLabel;
  }
  const param = new URLSearchParams(window.location.search).get("window");
  return param === "overlay" ||
    param === "overlay-control" ||
    param === "settings"
    ? param
    : "settings";
}
