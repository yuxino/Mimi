// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { getAppleTranslationStatus, profileCredentialEditorState } from "../../lib/ipc";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";
import { useAppleTranslationSupport } from "./useAppleTranslationSupport";

vi.mock("../../lib/ipc", async original => ({ ...await original<typeof import("../../lib/ipc")>(), getAppleTranslationStatus: vi.fn(), profileCredentialEditorState: vi.fn() }));
vi.mock("./useAppleTranslationSupport", async original => ({ ...await original<typeof import("./useAppleTranslationSupport")>(), useAppleTranslationSupport: vi.fn() }));
let host: HTMLDivElement, root: Root;
let props: ComponentProps<typeof AlibabaCredentialEditor>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  vi.mocked(getAppleTranslationStatus).mockReset().mockResolvedValue("installed");
  vi.mocked(profileCredentialEditorState).mockReset().mockResolvedValue({ savedFields: [] });
  vi.mocked(useAppleTranslationSupport).mockReturnValue({ support: { available: true, sourceLanguages: ["en", "ja"], targetLanguages: ["en", "ja"] }, loading: false, failed: false, refresh: vi.fn().mockResolvedValue(undefined) });
  props = { profile: { id: "test", name: "Apple", provider: "appleSpeech", credentialState: "present", speechCredentialState: "present", textCredentialState: "missing", textTranslation: "apple" },
    inputId: "apple-text", textOnly: true, sourceLanguage: "en", targetLanguage: "ja", disabled: false, busy: false, feedback: null, onSave: vi.fn().mockResolvedValue(null),
    confirmingDelete: false, onRequestDelete: vi.fn(), onConfirmDelete: vi.fn(), onCancelDelete: vi.fn() };
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });
const render = async (changes: Partial<typeof props> = {}) => { props = { ...props, ...changes }; await act(async () => root.render(<AlibabaCredentialEditor {...props} />)); };
const picker = () => host.querySelector<HTMLButtonElement>('.service-stage__selector [role="combobox"]')!;
async function choose(label: string) {
  await act(async () => picker().click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === label)!;
  await act(async () => option.click());
}

it.each(["appleSpeech", "customDashScopeASR", "customOpenAIASR"] as const)("shows credential-free Apple Translation for %s without credential reads", async provider => {
  await render({ profile: { ...props.profile, provider } });
  expect(picker().textContent).toContain(I18N.settings.textTranslationApple);
  expect(host.querySelectorAll("input, .saved-credential-input, .credential-storage-help")).toHaveLength(0);
  expect(profileCredentialEditorState).not.toHaveBeenCalled();
  expect(getAppleTranslationStatus).toHaveBeenCalledExactlyOnceWith("en", "ja");
  expect(host.querySelector('button[type="submit"]')).toBeNull();
});

it("offers a new Apple route only after native availability, preserving a saved unavailable route", async () => {
  vi.mocked(useAppleTranslationSupport).mockReturnValue({ support: { available: false, sourceLanguages: [], targetLanguages: [] }, loading: false, failed: false, refresh: vi.fn() });
  await render({ profile: { ...props.profile, textTranslation: "followService" } });
  await act(async () => picker().click());
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).not.toContain(I18N.settings.textTranslationApple);
  await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  await render({ profile: { ...props.profile, textTranslation: "apple" } });
  expect(picker().textContent).toContain(I18N.settings.textTranslationApple);
  expect(host.textContent).toContain(I18N.settings.appleTranslationUnavailable);
  expect(host.querySelector("input")).toBeNull();
  expect(getAppleTranslationStatus).not.toHaveBeenCalled();
});

it("drops a remote credential draft when saving Apple Translation", async () => {
  await render({ profile: { ...props.profile, textTranslation: "followService" } });
  await choose("DeepL");
  const input = host.querySelector<HTMLInputElement>('input[type="password"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "synthetic-secret-must-not-be-sent");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await choose(I18N.settings.textTranslationApple);
  expect(host.querySelector('input[type="password"]')).toBeNull();
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", textTranslation: "apple", apiKey: "", endpoint: "", token: "", model: "" });
});

it.each(["original", "en"] as const)("does not run translation checks for passthrough target %s", async targetLanguage => {
  const check = vi.fn();
  await render({ targetLanguage, textConnectionCheck: check });
  expect(check).not.toHaveBeenCalled();
  expect(getAppleTranslationStatus).not.toHaveBeenCalled();
  expect(host.textContent).toContain(targetLanguage === "original" ? I18N.settings.appleTranslationChooseTarget : I18N.settings.appleTranslationSameLanguage);
});

it.each(["zh", "en", "ja"] as const)("keeps the native translation guide with the section heading in %s", async language => {
  setStoredUiLanguage(language);
  await render({ disabled: true });
  const stage = host.querySelector(".service-stage--translation")!;
  const toggle = stage.querySelector<HTMLButtonElement>("header .apple-speech-tutorial-toggle")!;
  expect(toggle.textContent).toContain(I18N.settings.appleSpeechInstallHelp);
  expect(stage.querySelector(".apple-speech-tutorial")).toBeNull();
  await act(async () => toggle.click());
  expect(stage.querySelector("header")!.nextElementSibling?.className).toBe("apple-speech-tutorial");
  expect(stage.querySelectorAll(".apple-speech-tutorial ol li")).toHaveLength(3);
  expect(stage.querySelector(".apple-speech-tutorial")?.textContent).toContain(I18N.settings.appleTranslationDownloadHelp);
  expect(stage.querySelectorAll("h3")).toHaveLength(1);
});

it("groups target selection before preparation and keeps the translation check last", async () => {
  const languageControls = vi.fn(() => <div data-native-language-controls="true">Target picker</div>);
  const check = vi.fn(() => <button type="button">Check fixture</button>);
  await render({ translationLanguageControls: languageControls, textConnectionCheck: check });
  expect(languageControls).toHaveBeenCalledWith("apple");
  const stage = host.querySelector(".service-stage--translation")!;
  const controls = stage.querySelector("[data-native-language-controls]")!;
  expect(controls.nextElementSibling?.className).toBe("apple-translation-settings");
  expect(stage.lastElementChild?.className).toBe("apple-speech-connection-check");
  expect(stage.lastElementChild?.textContent).toBe("Check fixture");
  expect(stage.querySelector("header")?.textContent).not.toContain("Check fixture");
  await choose("DeepL");
  expect(stage.querySelector("[data-native-language-controls]")).toBeNull();
});
