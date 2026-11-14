import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "./lib/ipc";

export const settingsStartup = [performance.now(), 0, 0, 0, 0, 0];

export function loadSettingsView() {
  settingsStartup[2] = performance.now();
  return import("./windows/settings/SettingsView").then((module) => {
    settingsStartup[3] = performance.now();
    return { default: module.SettingsView };
  });
}

export function preloadSettingsView() {
  const experiment = window as Window & { __MIMI_PRELOAD_SETTINGS_VIEW__?: boolean };
  if (experiment.__MIMI_PRELOAD_SETTINGS_VIEW__ && isTauri && getCurrentWindow().label === "settings") {
    return loadSettingsView().then(module => module.default);
  }
  return undefined;
}
