// @vitest-environment jsdom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N } from "../../../lib/i18n";
import { SoftwareUpdate } from "./SoftwareUpdate";
import { createSoftwareUpdateSession } from "./softwareUpdateSession";
import { createFixtureSoftwareUpdater, type UpdateDownloadEvent } from "./softwareUpdater";
import { SettingsToastRegion } from "../SettingsToast";

vi.mock("../../../lib/ipc", () => ({ isTauri: false, appOpenReleases: vi.fn(), setOverlayPointerCursor: vi.fn() }));
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
  expect(host.querySelector(".software-update-button")?.textContent).toBe(I18N.settings.downloadUpdate);
  expect(host.querySelector('[role="status"]')?.textContent).toBe(I18N.settings.updateAvailable("9.9.9"));
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

it.each([true, false])("distinguishes transfer progress from signature verification (known size: %s)", async knownSize => {
  let emit!: (event: UpdateDownloadEvent) => void;
  let complete!: () => void;
  const download = new Promise<void>(resolve => { complete = resolve; });
  const updater = createFixtureSoftwareUpdater();
  const candidate = (await updater.check())!;
  vi.spyOn(updater, "check").mockResolvedValue({
    ...candidate,
    download: onEvent => { emit = onEvent; return download; },
  });
  const session = createSoftwareUpdateSession({ createEnvironment: async () => ({ kind: "installed", updater }) });
  await act(async () => root.render(<SoftwareUpdate session={session} />));
  await act(async () => host.querySelector<HTMLButtonElement>(".software-update-button")!.click());
  await act(async () => {
    emit({ event: "Started", data: { contentLength: knownSize ? 100 * 1_048_576 : undefined } });
    emit({ event: "Progress", data: { chunkLength: 42 * 1_048_576 } });
  });

  const progress = host.querySelector("progress")!;
  expect(progress.getAttribute("aria-label")).toBe(I18N.settings.downloadingUpdate);
  expect(progress.getAttribute("value")).toBe(knownSize ? "42" : null);
  expect(progress.getAttribute("aria-valuetext")).toBe(knownSize
    ? I18N.settings.downloadingKnown(42, "42.0 MB", "100.0 MB")
    : I18N.settings.downloadingUnknown("42.0 MB"));
  expect(host.textContent).toContain("42.0 MB");
  if (knownSize) expect(host.textContent).toContain("42%");
  else expect(host.textContent).not.toContain("%");

  // A finished transfer must remain busy until the native promise verifies it.
  await act(async () => {
    emit({ event: "Progress", data: { chunkLength: 58 * 1_048_576 } });
    emit({ event: "Finished" });
  });
  expect(progress.hasAttribute("value")).toBe(false);
  expect(progress.getAttribute("aria-label")).toBe(I18N.settings.verifyingUpdate);
  expect(progress.getAttribute("aria-valuetext")).toBe(I18N.settings.verifyingUpdate);
  expect(host.textContent).not.toContain("100%");
  expect(host.textContent).not.toContain(I18N.settings.updateDownloadVerified);
  expect(host.querySelector<HTMLButtonElement>(".software-update-button")!.disabled).toBe(true);

  await act(async () => complete());
  expect(host.querySelector("progress")).toBeNull();
  expect(host.textContent).toContain(I18N.settings.updateDownloadVerified);
  const install = host.querySelector<HTMLButtonElement>(".software-update-button")!;
  expect(install.disabled).toBe(false);
  expect(install.textContent).toBe(I18N.settings.installUpdate);
});
