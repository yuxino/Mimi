// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SessionErrorFeedback } from "./SessionErrorFeedback";
import { UI_LANGUAGES, setStoredUiLanguage } from "../lib/i18n";
import { systemAudioPermissionCopy } from "../lib/systemAudioPermissions";
import { selectSessionErrorMessage, selectSessionErrorSummary, useStore } from "../lib/store";
import permissionScopes from "../../src-tauri/permissions/app.toml?raw";
import nativeHandlers from "../../src-tauri/src/lib.rs?raw";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
let host: HTMLDivElement, root: Root;
const configure = vi.fn(), retry = vi.fn();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
  configure.mockReset(); retry.mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });

it.each(UI_LANGUAGES)("offers privacy settings and manual retry for the exact denied label in %s", async language => {
  setStoredUiLanguage(language);
  const state = { session: { ...useStore.getState().session, status: { kind: "error" as const, message: "System audio capture permission was denied." } } };
  const message = selectSessionErrorMessage(state)!;
  expect(message).toBe(systemAudioPermissionCopy().denied);
  await act(async () => root.render(<SessionErrorFeedback permissionRequired message={message} summary={selectSessionErrorSummary(state)!} onConfigure={configure} onRetry={retry} />));
  expect(invoke).not.toHaveBeenCalled(); expect(retry).not.toHaveBeenCalled();
  const buttons = host.querySelectorAll<HTMLButtonElement>("button");
  expect(buttons).toHaveLength(2);
  expect(buttons[0].textContent).toContain(systemAudioPermissionCopy().open);
  await act(async () => buttons[0].click());
  expect(invoke).toHaveBeenCalledExactlyOnceWith("app_open_audio_privacy_settings");
  expect(configure).not.toHaveBeenCalled(); expect(retry).not.toHaveBeenCalled();
  await act(async () => buttons[1].click()); expect(retry).toHaveBeenCalledOnce();
});

it("keeps a safe manual settings path if the OS opener fails", async () => {
  setStoredUiLanguage("zh");
  vi.mocked(invoke).mockRejectedValue(new Error("private-native-opener-detail"));
  await act(async () => root.render(<SessionErrorFeedback permissionRequired message={systemAudioPermissionCopy().denied} onConfigure={configure} onRetry={retry} />));
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(host.textContent).toContain(systemAudioPermissionCopy().failed);
  expect(host.textContent).not.toContain("private-native-opener-detail");
  expect(retry).not.toHaveBeenCalled();
});

it("authorizes the fixed privacy-page command for each bundled error surface", () => {
  expect(permissionScopes.split("[[permission]]").find(scope => scope.includes('identifier = "app-bootstrap"'))).toContain('"app_open_audio_privacy_settings"');
  expect(nativeHandlers).toContain("commands::app_open_audio_privacy_settings,");
});
