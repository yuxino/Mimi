// @vitest-environment jsdom
import { SettingsToastRegion } from "./SettingsToast";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import type { SettingsSnapshot } from "../../lib/types";
import { ProfileLanguageSettings } from "./ProfileLanguageSettings";

const initial = useStore.getState();
let host: HTMLDivElement, root: Root;
let save: ReturnType<typeof vi.fn>;
let settings: SettingsSnapshot;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  settings = { ...initial.settings, sourceLanguage: "auto", targetLanguage: "en", languageCapabilities: undefined,
    profiles: [{ id: "test", name: "Test", provider: "alibabaCloud", credentialState: "present" }], activeProfileId: "test" };
  save = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, settings, saveSettings: save }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});
async function render(disabled = false, requiresStop = false) {
  await act(async () => root.render(<><ProfileLanguageSettings settings={settings} disabled={disabled} requiresStop={requiresStop} /><SettingsToastRegion /></>));
}
async function choose(label: string, code: string) {
  const trigger = host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
  await act(() => trigger.click());
  const search = document.querySelector<HTMLInputElement>('input.mimi-select__search')!;
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, code);
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const option = document.querySelector<HTMLElement>('[role="option"]')!;
  await act(async () => option.click());
}

it("saves an explicit Chinese source without replacing the translation target", async () => {
  await render();
  await choose(I18N.settings.sourceLanguage, "zh");
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "zh" });
  expect(host.textContent).toContain(I18N.settings.languageSaved);
});

it.each(["fr", "zh_tw"])("saves a searched %s target with its exact wire key", async code => {
  settings = { ...settings, sourceLanguage: "zh" };
  await render();
  await choose(I18N.settings.translateTo, code);
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: code });
});

it("uses the broader recognition list only for an original-only route", async () => {
  settings = { ...settings, targetLanguage: "original" };
  await render();
  await choose(I18N.settings.sourceLanguage, "Norwegian");
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "no" });
});

it("guards duplicate saves and reports a safe failure without pretending the choice was saved", async () => {
  let fail!: (reason: Error) => void;
  save.mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  settings = { ...settings, profiles: [{ ...settings.profiles[0], provider: "openAIRealtime" }] };
  await render();
  const button = [...host.querySelectorAll<HTMLButtonElement>('[role="group"] button')].find(node => node.textContent === "Japanese")!;
  await act(() => { button.click(); button.click(); });
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: "ja" });
  expect(host.querySelector("section")?.getAttribute("aria-busy")).toBe("true");
  expect(button.disabled).toBe(true);
  await act(async () => fail(new Error("synthetic-private-provider-body")));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.languageSaveFailed);
  expect(host.textContent).not.toContain("synthetic-private-provider-body");
  expect(button.getAttribute("aria-pressed")).toBe("false");
  expect(button.disabled).toBe(false);
});

it("explains a session lock separately from a transient connection check", async () => {
  await render(true);
  expect(host.querySelectorAll('button:disabled')).toHaveLength(3);
  expect(host.textContent).not.toContain(I18N.settings.languageChangeRequiresStop);
  await render(true, true);
  expect(host.textContent).toContain(I18N.settings.languageChangeRequiresStop);
  expect(save).not.toHaveBeenCalled();
});

it("persists skipping translation as Original, restores the previous target, and keeps source-language help compact", async () => {
  await render();
  expect(host.querySelector('.settings-help-control__description')?.textContent).toBe(I18N.settings.recognitionLanguageHelp);
  const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.skipTranslation}"]`)!;
  await act(async () => toggle().click());
  expect(save).toHaveBeenCalledWith({ targetLanguage: "original" });
  settings = { ...settings, targetLanguage: "original" }; await render();
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  expect(host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.settings.translateTo}"]`)!.disabled).toBe(true);
  await act(async () => toggle().click());
  expect(save).toHaveBeenLastCalledWith({ targetLanguage: "en" });
});
