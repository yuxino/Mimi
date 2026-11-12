// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { OPENAI_COMPATIBLE_PRESETS } from "../../lib/generated/providerCopy";
import { PROVIDER_COPY as EN_PROVIDER_COPY } from "../../lib/generated/providerCopy/en";
import { setStoredUiLanguage, UI_LANGUAGES } from "../../lib/i18n";
import { openTextTranslationSetupPage } from "../../lib/ipc";
import { SettingsToastRegion } from "./SettingsToast";
import { TextTranslationSetupLink } from "./TextTranslationSetupLink";

vi.mock("../../lib/ipc", () => ({ isTauri: false, openTextTranslationSetupPage: vi.fn() }));
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  vi.mocked(openTextTranslationSetupPage).mockReset().mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  setStoredUiLanguage("en"); vi.unstubAllGlobals();
});

it.each(OPENAI_COMPATIBLE_PRESETS)("opens only explicit catalog actions for $label", async preset => {
  await act(async () => root.render(<>
    <TextTranslationSetupLink preset={preset} page="apiKey" />
    <TextTranslationSetupLink preset={preset} page="guide" />
  </>));
  expect(openTextTranslationSetupPage).not.toHaveBeenCalled();
  const buttons = [...host.querySelectorAll<HTMLButtonElement>("button")];
  expect(buttons).toHaveLength(preset.apiKeyUrl ? 2 : 1);
  if (preset.apiKeyUrl) {
    await act(async () => buttons[0].click());
    expect(openTextTranslationSetupPage).toHaveBeenLastCalledWith(preset.id, "apiKey");
  }
  await act(async () => buttons.at(-1)!.click());
  expect(openTextTranslationSetupPage).toHaveBeenLastCalledWith(preset.id, "guide");
  expect(host.querySelector("p")).toBeNull();
});

it.each(UI_LANGUAGES)("labels local setup without a cloud-key action in %s", async language => {
  setStoredUiLanguage(language);
  const preset = OPENAI_COMPATIBLE_PRESETS.find(item => item.id === "ollama")!;
  await act(async () => root.render(<><TextTranslationSetupLink preset={preset} page="apiKey" /><TextTranslationSetupLink preset={preset} page="guide" /></>));
  expect(host.querySelectorAll("button")).toHaveLength(1);
  expect(host.textContent).toBe((await import(`../../lib/generated/providerCopy/${language}.ts`)).PROVIDER_COPY.compatibleSetupLocalGuide);
});

it("shows a sanitized transient failure and keeps disabled actions inactive", async () => {
  const preset = OPENAI_COMPATIBLE_PRESETS[0];
  vi.mocked(openTextTranslationSetupPage).mockRejectedValue(new Error("private-browser-detail"));
  await act(async () => root.render(<><TextTranslationSetupLink preset={preset} page="apiKey" /><SettingsToastRegion /></>));
  await act(async () => host.querySelector<HTMLButtonElement>(".translation-setup-link")!.click());
  expect(host.querySelector(".settings-toast")?.textContent).toContain(EN_PROVIDER_COPY.compatibleSetupOpenFailed);
  expect(host.textContent).not.toContain("private-browser-detail");
  vi.mocked(openTextTranslationSetupPage).mockClear();
  await act(async () => root.render(<TextTranslationSetupLink preset={preset} page="apiKey" disabled />));
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(openTextTranslationSetupPage).not.toHaveBeenCalled();
});
