// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { openTencentSetupPage } from "../../lib/ipc";
import { SettingsToastRegion } from "./SettingsToast";
import { TencentSetupHelp } from "./TencentSetupHelp";

vi.mock("../../lib/ipc", () => ({ isTauri: false, openTencentSetupPage: vi.fn(), setOverlayPointerCursor: vi.fn() }));
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(openTencentSetupPage).mockReset().mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  setStoredUiLanguage("en"); vi.unstubAllGlobals();
});

it.each(["zh", "en", "ja"] as const)("opens only explicit Tencent setup destinations with compact help in %s", async language => {
  setStoredUiLanguage(language);
  await act(async () => root.render(<TencentSetupHelp />));
  expect(host.querySelector(".settings-help-control__description")?.textContent).toBe(I18N.settings.tencentSetupHelp);
  expect(host.querySelector("p")).toBeNull();
  expect(openTencentSetupPage).not.toHaveBeenCalled();
  const buttons = host.querySelectorAll<HTMLButtonElement>(".tencent-setup-help__actions button");
  expect(buttons).toHaveLength(3);
  for (const [index, destination] of ["account", "apiKey", "asr"].entries()) {
    await act(async () => buttons[index].click());
    expect(openTencentSetupPage).toHaveBeenLastCalledWith(destination);
  }
});

it("reports console-opening failures with a sanitized transient toast", async () => {
  vi.mocked(openTencentSetupPage).mockRejectedValue(new Error("private-native-detail"));
  await act(async () => root.render(<><TencentSetupHelp /><SettingsToastRegion /></>));
  await act(async () => host.querySelector<HTMLButtonElement>(".tencent-setup-help__actions button")!.click());
  expect(host.textContent).toContain(I18N.settings.tencentSetupOpenFailed);
  expect(host.textContent).not.toContain("private-native-detail");
});
