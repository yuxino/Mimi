import { StrictMode, type ComponentType } from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { effectiveUiLanguage } from "./lib/i18n";
import { isTauri } from "./lib/ipc";
import "./index.css";
import "./linux.css";

performance.mark("mimi-startup-entry");

// Cosmetic platform styling must be ready before the first window paints.
// This is not used to select native capabilities or permission behavior.
const userAgent = navigator.userAgent;
if (/\bLinux\b/i.test(userAgent) && !/\b(Android|CrOS)\b/i.test(userAgent)) {
  document.documentElement.dataset.platform = "linux";
}
const uiLanguage = effectiveUiLanguage();
document.documentElement.lang = uiLanguage === "zh" ? "zh-CN" : uiLanguage;

function mount(settingsView?: ComponentType) {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <StrictMode>
      <App settingsView={settingsView} />
    </StrictMode>
  );
}

function preloadSettings() {
  // Load the existing Settings chunk before the first render to avoid the
  // initial Suspense fallback and its delayed retry on Windows WebView2.
  void import("./windows/settings/SettingsView").then(
    ({ SettingsView }) => {
      performance.mark("mimi-startup-settings-imported");
      mount(SettingsView);
    },
    () => mount(),
  );
}

if (__MIMI_WINDOWS_BUILD__ && isTauri && getCurrentWindow().label === "settings") {
  preloadSettings();
} else {
  mount();
}
