// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N } from "../../lib/i18n";
import { appOpenReleases } from "../../lib/ipc";
import { SoftwareUpdate } from "./SoftwareUpdate";
import { SettingsToastRegion } from "./SettingsToast";
import { createUpdaterForEnvironment } from "./softwareUpdateEnvironment";

vi.mock("./softwareUpdateEnvironment", () => ({ createUpdaterForEnvironment: vi.fn() }));
vi.mock("../../lib/ipc", () => ({ isTauri: false, appOpenReleases: vi.fn(), setOverlayPointerCursor: vi.fn() }));
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(appOpenReleases).mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
async function render() { await act(async () => root.render(<><SoftwareUpdate /><SettingsToastRegion /></>)); }
async function openReleases() { await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === I18N.settings.openReleaseRecovery)!.click()); }

it.each(["portable", "linuxPackage"] as const)("reports %s Releases-open failure as a toast and allows retry", async kind => {
  vi.mocked(createUpdaterForEnvironment).mockResolvedValue({ kind, currentVersion: "fixture" });
  vi.mocked(appOpenReleases).mockRejectedValueOnce(new Error("private launch failure")).mockResolvedValue(undefined);
  await render(); await openReleases();
  expect(host.querySelector('.settings-toast[role="alert"]')?.textContent).toBe(I18N.settings.openUpdateFailed);
  expect(host.querySelector(".software-update [role=alert]")).toBeNull();
  expect(host.textContent).not.toContain("private launch failure");
  await openReleases();
  expect(appOpenReleases).toHaveBeenCalledTimes(2);
  expect(host.querySelector(".settings-toast")).toBeNull();
});

it("keeps the updater failure actionable while its separate recovery-link failure uses a toast", async () => {
  vi.mocked(createUpdaterForEnvironment).mockRejectedValue(new Error("fixture environment failure"));
  vi.mocked(appOpenReleases).mockRejectedValue(new Error("private browser failure"));
  await render(); await openReleases();
  expect(host.querySelector(".software-update .settings-feedback")?.textContent).toBe(I18N.settings.updateCheckFailed);
  expect(host.querySelector(".settings-toast")?.textContent).toBe(I18N.settings.openUpdateFailed);
  expect(host.querySelector(".software-update__recovery [role=alert]")).toBeNull();
  expect(host.querySelector<HTMLButtonElement>(".software-update__recovery button")!.disabled).toBe(false);
});
