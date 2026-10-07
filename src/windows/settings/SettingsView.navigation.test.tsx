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
vi.mock("./ServiceProfiles", () => ({ ServiceProfiles: ({ appleResourcesRequest, profileEditorRequest }: { appleResourcesRequest: number; profileEditorRequest: number }) => <div data-apple-resource-request={appleResourcesRequest} data-profile-editor-request={profileEditorRequest} /> }));
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

it("opens subtitles when an existing configured profile arrives after the empty native placeholder", async () => {
  window.history.replaceState(null, "", window.location.pathname);
  useStore.setState({ initializationStatus: "loading" });
  await act(async () => root.render(<SettingsView />));
  expect(host.querySelector("#settings-category-service")?.getAttribute("aria-current")).toBe("page");
  await act(async () => useStore.setState({ initializationStatus: "ready", settings: { ...initial.settings,
    activeProfileId: "existing", profiles: [{ id: "existing", provider: "googleGeminiLive", name: "Existing service", credentialState: "present" }] } }));
  expect(host.querySelector("#settings-category-subtitles")?.getAttribute("aria-current")).toBe("page");
});

it("keeps a fresh install on services and preserves navigation after its first profile is added", async () => {
  window.history.replaceState(null, "", window.location.pathname);
  await act(async () => root.render(<SettingsView />));
  expect(host.querySelector("#settings-category-service")?.getAttribute("aria-current")).toBe("page");
  await act(async () => host.querySelector<HTMLButtonElement>("#settings-category-subtitles")!.click());
  expect(host.querySelector("#settings-session-status")?.textContent).toBe(I18N.settings.sessionSetupRequired);
  await act(async () => host.querySelector<HTMLButtonElement>("#settings-category-general")!.click());
  await act(async () => useStore.setState({ settings: { ...initial.settings,
    activeProfileId: "first", profiles: [{ id: "first", provider: "googleGeminiLive", name: "First service", credentialState: "present" }] } }));
  expect(host.querySelector("#settings-category-general")?.getAttribute("aria-current")).toBe("page");
});

it("routes repeated Apple resource intents to the service page without losing the specific destination", async () => {
  await act(async () => root.render(<SettingsView />));
  await act(async () => navigate("appleSpeechResources"));
  expect(host.querySelector("#settings-category-service")?.getAttribute("aria-current")).toBe("page");
  expect(host.querySelector("[data-apple-resource-request]")?.getAttribute("data-apple-resource-request")).toBe("1");
  await act(async () => navigate("export"));
  expect(host.querySelector("[data-apple-resource-request]")?.getAttribute("data-apple-resource-request")).toBe("0");
  await act(async () => navigate("appleSpeechResources"));
  expect(host.querySelector("#settings-category-service")?.getAttribute("aria-current")).toBe("page");
  expect(host.querySelector("[data-apple-resource-request]")?.getAttribute("data-apple-resource-request")).toBe("2");
});

it("routes repeated current-configuration intents to the detail request without focusing category navigation", async () => {
  await act(async () => root.render(<SettingsView />));
  await act(async () => navigate("activeProfile"));
  expect(host.querySelector("#settings-category-service")?.getAttribute("aria-current")).toBe("page");
  expect(host.querySelector("[data-profile-editor-request]")?.getAttribute("data-profile-editor-request")).toBe("1");
  await act(async () => navigate("export"));
  expect(host.querySelector("[data-profile-editor-request]")?.getAttribute("data-profile-editor-request")).toBe("0");
  await act(async () => navigate("activeProfile"));
  expect(host.querySelector("[data-profile-editor-request]")?.getAttribute("data-profile-editor-request")).toBe("2");
  expect(host.querySelector("[data-apple-resource-request]")?.getAttribute("data-apple-resource-request")).toBe("0");
});
