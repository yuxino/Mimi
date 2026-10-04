// @vitest-environment jsdom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N } from "../../lib/i18n";
import { SoftwareUpdate } from "./SoftwareUpdate";
import { createSoftwareUpdateSession } from "./softwareUpdateSession";
import { createFixtureSoftwareUpdater } from "./softwareUpdater";
import { SettingsToastRegion } from "./SettingsToast";

vi.mock("../../lib/ipc", () => ({ isTauri: false, appOpenReleases: vi.fn(), setOverlayPointerCursor: vi.fn() }));
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("checks on General entry only and keeps no-update results quiet on reentry", async () => {
  const updater = createFixtureSoftwareUpdater({ updateVersion: null });
  const check = vi.spyOn(updater, "check");
  const session = createSoftwareUpdateSession({ createEnvironment: async () => ({ kind: "installed", updater }) });
  const render = (active: boolean) => act(async () => root.render(<StrictMode><SoftwareUpdate active={active} session={session} /><SettingsToastRegion /></StrictMode>));
  await render(false); expect(check).not.toHaveBeenCalled();
  await render(true); expect(check).toHaveBeenCalledOnce();
  expect(host.textContent).toContain(I18N.settings.noUpdateAvailable);
  expect(host.querySelector(".settings-toast")).toBeNull();
  await render(false); await render(true); expect(check).toHaveBeenCalledOnce();
  await act(async () => host.querySelector<HTMLButtonElement>(".software-update-button")!.click());
  expect(check).toHaveBeenCalledTimes(2);
  expect(host.querySelector(".settings-toast")?.textContent).toBe(I18N.settings.noUpdateAvailable);
});

it("keeps an automatic failure quiet and offers an enabled initialization retry", async () => {
  const createEnvironment = vi.fn().mockRejectedValueOnce(new Error("private path"))
    .mockResolvedValue({ kind: "installed", updater: createFixtureSoftwareUpdater() });
  const session = createSoftwareUpdateSession({ createEnvironment });
  await act(async () => root.render(<><SoftwareUpdate session={session} /><SettingsToastRegion /></>));
  expect(host.querySelector(".settings-toast")).toBeNull();
  const retry = host.querySelector<HTMLButtonElement>(".software-update-button")!;
  expect(retry.disabled).toBe(false); expect(retry.textContent).toBe(I18N.settings.retryUpdate);
  expect(host.textContent).not.toContain("private path");
  await act(async () => retry.click());
  expect(host.textContent).toContain(I18N.settings.updateAvailable("9.9.9"));
});

it("renders localized notes and retains the verified package through remount", async () => {
  const updater = createFixtureSoftwareUpdater({ notes: "## English\n- **Readable** notes\n## 中文\n- **可读**说明", installError: new Error("private install error") });
  const check = vi.spyOn(updater, "check");
  const session = createSoftwareUpdateSession({ createEnvironment: async () => ({ kind: "installed", updater }) });
  const render = () => act(async () => root.render(<SoftwareUpdate session={session} />));
  await render();
  expect(host.querySelector("details")?.open).toBe(false);
  expect(host.querySelector(".software-update__notes strong")).not.toBeNull();
  expect(host.querySelector(".software-update__notes")?.textContent).not.toContain("##");
  await act(async () => host.querySelector<HTMLButtonElement>(".software-update-button")!.click());
  expect(host.textContent).toContain(I18N.settings.updateDownloadVerified);
  expect(host.querySelector(".settings-feedback")).toBeNull();
  await act(async () => root.render(null)); await render();
  expect(check).toHaveBeenCalledOnce();
  expect(host.querySelector(".software-update-button")?.textContent).toBe(I18N.settings.installUpdate);
  await act(async () => host.querySelector<HTMLButtonElement>(".software-update-button")!.click());
  expect(host.textContent).toContain(I18N.settings.updateInstallFailed);
  expect(host.textContent).not.toContain("private install error");
  expect(host.querySelector(".software-update-button")?.textContent).toBe(I18N.settings.retryUpdate);
});
