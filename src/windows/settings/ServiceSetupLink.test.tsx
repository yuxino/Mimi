// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SERVICE_SETUPS } from "../../lib/generated/providerCopy";
import { PROVIDER_COPY as EN_PROVIDER_COPY } from "../../lib/generated/providerCopy/en";
import { setStoredUiLanguage, UI_LANGUAGES } from "../../lib/i18n";
import { openServiceSetupPage } from "../../lib/ipc";
import { SettingsToastRegion } from "./SettingsToast";
import { ServiceSetupLink } from "./ServiceSetupLink";

vi.mock("../../lib/ipc", () => ({ isTauri: false, openServiceSetupPage: vi.fn() }));
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  vi.mocked(openServiceSetupPage).mockReset().mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  setStoredUiLanguage("en"); vi.unstubAllGlobals();
});

it.each(SERVICE_SETUPS)("opens $label only after an explicit click", async service => {
  await act(async () => root.render(<ServiceSetupLink serviceId={service.id} />));
  expect(openServiceSetupPage).not.toHaveBeenCalled();
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(openServiceSetupPage).toHaveBeenCalledExactlyOnceWith(service.id, "apiKey");
});

it.each(UI_LANGUAGES)("keeps the key action readable in %s", async language => {
  setStoredUiLanguage(language);
  await act(async () => root.render(<ServiceSetupLink serviceId="openAIRealtime" />));
  expect(host.textContent).toBe((await import(`../../lib/generated/providerCopy/${language}.ts`)).PROVIDER_COPY.compatibleSetupGetKey);
});

it("hides unknown/custom services and reports sanitized opening failures", async () => {
  await act(async () => root.render(<ServiceSetupLink serviceId="customOpenAIASR" />));
  expect(host.querySelector("button")).toBeNull();
  vi.mocked(openServiceSetupPage).mockRejectedValue(new Error("private-browser-detail"));
  await act(async () => root.render(<><ServiceSetupLink serviceId="openAIRealtime" /><SettingsToastRegion /></>));
  await act(async () => host.querySelector<HTMLButtonElement>(".translation-setup-link")!.click());
  expect(host.querySelector(".settings-toast")?.textContent).toContain(EN_PROVIDER_COPY.compatibleSetupOpenFailed);
  expect(host.textContent).not.toContain("private-browser-detail");
  vi.mocked(openServiceSetupPage).mockClear();
  await act(async () => root.render(<ServiceSetupLink serviceId="openAIRealtime" disabled />));
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(openServiceSetupPage).not.toHaveBeenCalled();
});
