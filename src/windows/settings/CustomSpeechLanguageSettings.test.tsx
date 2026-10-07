// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, SOURCE_LANGUAGE_CODES, type ServiceProfile, type SourceLanguage } from "../../lib/types";
import { CustomSpeechLanguageSettings } from "./CustomSpeechLanguageSettings";

let host: HTMLDivElement, root: Root, profile: ServiceProfile;
let save: ReturnType<typeof vi.fn>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  profile = { id: "custom", name: "Any model", provider: "customDashScopeASR", credentialState: "missing" };
  save = vi.fn().mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });
const render = async (disabled = false) => { await act(async () => root.render(<CustomSpeechLanguageSettings profile={profile} disabled={disabled} onSave={save} />)); };
const button = (label: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === label)!;
const language = (code: SourceLanguage) => button(SOURCE_LANGUAGE_DISPLAY_NAMES[code] + code);
async function click(label: string) { await act(async () => button(label).click()); }
async function mode(label: string) {
  await act(async () => host.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
  await act(async () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent === label)!.click());
}
async function search(value: string) {
  const input = host.querySelector<HTMLInputElement>('input[type="search"]')!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
}
it.each(["zh", "en", "ja"] as const)("keeps choices collapsed until editing and saves declared codes in %s", async locale => {
  setStoredUiLanguage(locale);
  await render();
  expect(host.querySelector(".custom-speech-languages__choices")).toBeNull();
  expect(host.textContent).toContain(I18N.settings.customSpeechLanguagesUnknown);
  await click(I18N.settings.customSpeechLanguagesEdit);
  await mode(I18N.settings.customSpeechLanguagesManual);
  expect(host.querySelectorAll(".custom-speech-languages__choices button")).toHaveLength(SOURCE_LANGUAGE_CODES.length);
  expect(host.querySelector('[aria-pressed="true"]')).toBeNull();
  await search("fr");
  expect(language("fr")).toBeDefined();
  await act(async () => language("fr").click());
  expect(language("fr").classList.contains("is-selected")).toBe(true);
  expect(language("fr").getAttribute("aria-pressed")).toBe("true");
  await click(I18N.settings.customSpeechLanguagesSave);
  expect(save).toHaveBeenCalledExactlyOnceWith(["fr"]);
  expect(host.querySelector(".custom-speech-languages__expanded")).toBeNull();
});
it("distinguishes default-only from clearing the declaration and cancels unsaved edits", async () => {
  profile.customSpeechSourceLanguages = ["en"];
  await render(); await click(I18N.settings.customSpeechLanguagesEdit);
  await act(async () => language("en").click());
  expect(language("en").classList.contains("is-selected")).toBe(false);
  expect(host.textContent).toContain(I18N.settings.customSpeechLanguagesDefaultOnly);
  await click(I18N.settings.customSpeechLanguagesSave);
  expect(save).toHaveBeenLastCalledWith([]);
  profile = { ...profile, customSpeechSourceLanguages: [] };
  await render(); await click(I18N.settings.customSpeechLanguagesEdit);
  await mode(I18N.settings.customSpeechLanguagesUnknown);
  await click(I18N.settings.customSpeechLanguagesSave);
  expect(save).toHaveBeenLastCalledWith(null);
  profile = { ...profile, customSpeechSourceLanguages: null };
  await render(); await click(I18N.settings.customSpeechLanguagesEdit);
  await mode(I18N.settings.customSpeechLanguagesManual);
  await act(async () => language("en").click());
  await click(I18N.settings.cancel);
  expect(save).toHaveBeenCalledTimes(2);
  expect(host.querySelector(".custom-speech-languages__summary")?.textContent).toBe(I18N.settings.customSpeechLanguagesUnknown);
});
it("keeps a failed draft, prevents duplicate requests, and retries without showing provider errors", async () => {
  let reject!: (error: Error) => void;
  save.mockImplementationOnce(() => new Promise((_, failure) => { reject = failure; }));
  await render(); await click(I18N.settings.customSpeechLanguagesEdit); await mode(I18N.settings.customSpeechLanguagesManual);
  await act(async () => language("en").click());
  const trigger = button(I18N.settings.customSpeechLanguagesSave);
  await act(async () => { trigger.click(); trigger.click(); });
  expect(save).toHaveBeenCalledTimes(1);
  expect(trigger.disabled).toBe(true);
  expect(language("en").disabled).toBe(true);
  await act(async () => reject(new Error("synthetic-private-error")));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.customSpeechLanguagesSaveFailed);
  expect(host.textContent).not.toContain("synthetic-private-error");
  expect(language("en").classList.contains("is-selected")).toBe(true);
  await click(I18N.settings.customSpeechLanguagesSave);
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith(["en"]);
});
it("disables editing while a session is active", async () => {
  await render(true);
  expect(button(I18N.settings.customSpeechLanguagesEdit).disabled).toBe(true);
  await click(I18N.settings.customSpeechLanguagesEdit);
  expect(host.querySelector(".custom-speech-languages__expanded")).toBeNull();
  expect(save).not.toHaveBeenCalled();
});

it("retains explicitly declared non-Alibaba languages and regional variants", async () => {
  profile.customSpeechSourceLanguages = ["uk"];
  await render(); await click(I18N.settings.customSpeechLanguagesEdit);
  expect(language("uk").getAttribute("aria-pressed")).toBe("true");
  expect(language("pt-BR").getAttribute("aria-pressed")).toBe("false");
  await search("pt-br");
  expect(language("pt-BR")).toBeDefined();
  await act(async () => language("pt-BR").click());
  await click(I18N.settings.customSpeechLanguagesSave);
  expect(save).toHaveBeenCalledExactlyOnceWith(["pt-BR", "uk"]);
});
