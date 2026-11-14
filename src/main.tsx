import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import type { ComponentType } from "react";
import { preloadSettingsView } from "./settingsStartupProbe";
import { effectiveUiLanguage } from "./lib/i18n";
import "./index.css";
import "./linux.css";

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
    <React.StrictMode>
      <App settingsView={settingsView} />
    </React.StrictMode>
  );
}

const preloadedSettings = preloadSettingsView();
if (preloadedSettings) void preloadedSettings.then(mount, () => mount());
else mount();
