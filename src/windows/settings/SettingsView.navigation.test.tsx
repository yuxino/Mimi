// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import type { SettingsNavigationTarget } from "../../lib/ipc";
import { SettingsView } from "./SettingsView";

const native = vi.hoisted(() => ({ listen: vi.fn(), ready: vi.fn(), archive: vi.fn(), unlisten: vi.fn() }));
vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(), isTauri: true,
  listenSettingsNavigation: native.listen, announceSettingsNavigationReady: native.ready,
  appDesktopShortcutCommands: vi.fn().mockResolvedValue(null),
  sessionArchiveState: native.archive, sessionHistoryList: vi.fn().mockResolvedValue([]),
}));
// These unrelated pages do not participate in the native navigation contract.
vi.mock("./ServiceProfiles", () => ({ ServiceProfiles: () => null }));
vi.mock("./SoftwareUpdate", () => ({ SoftwareUpdate: () => null }));
vi.mock("./WindowsAudioSource", () => ({ WindowsAudioSource: () => null }));
vi.mock("./SupportDiagnostics", () => ({ SupportDiagnostics: () => null }));
vi.mock("./QuickStartGuide", () => ({ QuickStartGuide: () => null }));

const initial = useStore.getState();
let host: HTMLDivElement, root: Root;
let navigate!: (target: SettingsNavigationTarget) => void;
beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", () => {});
  Element.prototype.scrollTo = vi.fn();
  setStoredUiLanguage("en"); window.history.replaceState(null, "", "#subtitle-settings");
  native.listen.mockReset().mockImplementation((handler: typeof navigate) => { navigate = handler; return Promise.resolve(native.unlisten); });
  native.ready.mockReset().mockResolvedValue(undefined); native.unlisten.mockReset();
  native.archive.mockReset().mockResolvedValue({ transcriptCount: 0, transcriptLimited: false, audioBytes: 0, audioLimited: false, sampleRate: null, historySaveError: true });
  useStore.setState({ ...initial, initializationStatus: "ready", hasSettingsSnapshot: true,
    settings: { ...initial.settings, profiles: initial.settings.profiles.map(profile => ({ ...profile, credentialState: "present" })) },
    session: { ...initial.session, status: { kind: "idle" }, isActive: false, isPaused: false },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(() => root.unmount()); host.remove(); useStore.setState(initial, true);
  setStoredUiLanguage("system"); window.history.replaceState(null, "", window.location.pathname); vi.unstubAllGlobals(); vi.useRealTimers();
});

it("routes a failed native quit to export, focuses its nav action and shows the existing safe save failure beside a reachable retry", async () => {
  const quit = vi.fn().mockRejectedValue(new Error("synthetic quit failure"));
  useStore.setState({ quit });
  await act(async () => root.render(<SettingsView />));
  expect(native.ready).toHaveBeenCalledOnce();
  await act(async () => navigate("export"));
  const exportNavigation = host.querySelector<HTMLButtonElement>("#settings-category-export")!;
  expect(exportNavigation.getAttribute("aria-current")).toBe("page");
  expect(document.activeElement).toBe(exportNavigation);
  expect(window.location.hash).toBe("#session-export");
  expect(host.querySelector("#session-export [role=alert]")?.textContent).toBe(I18N.settings.historySaveFailed);
  const retry = host.querySelector<HTMLButtonElement>(".settings-quit-button")!;
  expect(retry.disabled).toBe(false);
  await act(async () => retry.click());
  expect(quit).toHaveBeenCalledOnce();
  expect(host.querySelector('.settings-toast[role="alert"]')?.textContent).toBe(I18N.tray.quitFailed);
  expect(host.querySelector(".settings-sidebar .settings-feedback, .settings-quit-error")).toBeNull();
  expect(host.textContent).not.toContain("synthetic quit failure");
});

it("preserves the existing service navigation intent", async () => {
  await act(async () => root.render(<SettingsView />));
  await act(async () => navigate("service"));
  const service = host.querySelector<HTMLButtonElement>("#settings-category-service")!;
  expect(service.getAttribute("aria-current")).toBe("page");
  expect(document.activeElement).toBe(service);
  expect(window.location.hash).toBe("#service-profiles");
});
