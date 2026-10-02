// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { diagnosticCopy } from "../../lib/connectionDiagnostics";
import { profileRevealCredential } from "../../lib/ipc";
import type { ServiceProfile, TextTranslation } from "../../lib/types";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";

vi.mock("../../lib/ipc", () => ({ isTauri: false, profileRevealCredential: vi.fn(), setOverlayPointerCursor: vi.fn() }));

let root: Root;
let host: HTMLDivElement;
let props: Parameters<typeof AlibabaCredentialEditor>[0];
const profile: ServiceProfile = { id: "synthetic", name: "Alibaba", provider: "alibabaCloud", credentialState: "present" };
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("scrollIntoView", vi.fn());
  Element.prototype.scrollIntoView = vi.fn();
  vi.mocked(profileRevealCredential).mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  props = { profile, inputId: "test", disabled: false, busy: false, feedback: null, onSave: vi.fn().mockResolvedValue(null), onRequestDelete: vi.fn(), onConfirmDelete: vi.fn(), confirmingDelete: false, onCancelDelete: vi.fn() };
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); setStoredUiLanguage("en"); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function render(next = props) { props = next; await act(() => root.render(<AlibabaCredentialEditor {...props} />)); }
async function change(selector: string, value: string) {
  const node = host.querySelector<HTMLInputElement>(selector)!;
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(node, value);
    node.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
function picker() { return host.querySelector<HTMLButtonElement>('.service-stage--translation [role="combobox"]')!; }
async function chooseTranslation(value: TextTranslation) {
  await act(() => picker().click());
  const label = value === "openAICompatible" ? I18N.settings.textTranslationOpenAICompatible : value === "deepLX" ? I18N.settings.textTranslationCustom : value === "deepL" ? "DeepL" : I18N.settings.textTranslationFollow;
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === label)!;
  await act(() => option.click());
}
async function key(value: string) {
  await act(() => picker().dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true })));
}
async function submit() { await act(async () => { host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); }); }

it("separates recognition and translation during first-time setup while requiring only the recognition key", async () => {
  await render({ ...props, profile: { ...profile, credentialState: "missing" } });
  expect(host.querySelector("details")).toBeNull();
  expect([...host.querySelectorAll(".service-stage h3")].map(node => node.textContent)).toEqual([I18N.settings.speechRecognition, I18N.settings.textTranslationLabel]);
  expect(host.querySelector('.service-stage:not(.service-stage--translation) .provider-icon[data-provider="alibabaCloud"]')).not.toBeNull();
  expect(host.querySelectorAll(".service-stage--translation [role=combobox]")).toHaveLength(1);
  expect(host.querySelectorAll("input")).toHaveLength(1);
  expect(host.querySelector("select")).toBeNull();
  expect(picker().textContent).toBe(I18N.settings.textTranslationFollow);
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
  await change("input", "synthetic-asr"); await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "alibabaTranslation", model: "", apiKey: "synthetic-asr", textTranslation: "followService", endpoint: "", token: "" });
});

it("configures only the text translation destination while reusing a saved recognition key", async () => {
  await render();
  await chooseTranslation("deepLX");
  expect(host.textContent).toContain(I18N.settings.deepLXChain);
  expect(host.querySelector('input[id="test-apiKey"]')).toBeNull();
  await change("#test-endpoint", "https://example.com/translate"); await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "alibabaTranslation", model: "", apiKey: "", textTranslation: "deepLX", endpoint: "https://example.com/translate", token: "" });
});

it("retains invalid input and focuses the adjacent error before any credential call", async () => {
  await render({ ...props, profile: { ...profile, credentialState: "missing" } });
  await change("input", "synthetic-asr"); await chooseTranslation("deepLX"); await change("#test-endpoint", "bad"); await submit();
  expect(props.onSave).not.toHaveBeenCalled();
  expect((host.querySelector("#test-apiKey") as HTMLInputElement).value).toBe("synthetic-asr");
  expect((host.querySelector("#test-endpoint") as HTMLInputElement).value).toBe("bad");
  expect(document.activeElement?.id).toBe("test-endpoint");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.deepLXEndpointInvalid);
  expect(Element.prototype.scrollIntoView).toHaveBeenCalledWith({ block: "center" });
  await change("#test-endpoint", "http://localhost:1188");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("preserves an unsaved key when saving succeeds but activation fails", async () => {
  await render({ ...props, profile: { ...profile, credentialState: "missing" } });
  await change("input", "synthetic-asr"); await submit();
  await render({ ...props, profile });
  expect((host.querySelector("#test-apiKey") as HTMLInputElement).value).toBe("synthetic-asr");
});

it("retains the historical DeepLX route and switches back without repeating the key", async () => {
  await render({ ...props, profile: { ...profile, provider: "deepLX" } });
  expect(picker().textContent).toBe(I18N.settings.textTranslationCustom);
  expect(picker().querySelector('.provider-icon[data-provider="deepLX"]')).not.toBeNull();
  expect(host.querySelector('.service-stage:not(.service-stage--translation) .provider-icon[data-provider="alibabaCloud"]')).not.toBeNull();
  expect(host.textContent).toContain(I18N.settings.deepLXChain);
  expect(host.querySelector('input[type="password"]')?.getAttribute("id")).toBe("test-token");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.placeholder).toBe(I18N.settings.savedServiceAddressPlaceholder);
  await chooseTranslation("followService"); await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "alibabaTranslation", model: "", apiKey: "", textTranslation: "followService", endpoint: "", token: "" });
});

it.each(["en", "zh", "ja"] as const)("shows both stages in read-only file mode without exposing any credential editor in %s", async (language) => {
  setStoredUiLanguage(language);
  await render({ ...props, readOnly: true, profile: { ...profile, textTranslation: "openAICompatible" } });
  expect([...host.querySelectorAll(".service-stage h3")].map(node => node.textContent)).toEqual([I18N.settings.speechRecognition, I18N.settings.textTranslationLabel]);
  expect(picker().textContent).toBe(I18N.settings.textTranslationOpenAICompatible);
  expect(picker().disabled).toBe(true);
  expect(host.querySelector(".credential-form, input, .stored-credential-reveal, .credential-panel__saved-actions, .credential-form__actions, button[type=submit]")).toBeNull();
  expect(host.querySelector(".service-credential-toolbar .settings-help-control__description")?.textContent).toBe(diagnosticCopy().localDevReadOnly);
  expect(host.querySelector(".service-credential-toolbar p")).toBeNull();
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(props.onSave).not.toHaveBeenCalled();
  expect(props.onRequestDelete).not.toHaveBeenCalled();
  expect(props.onConfirmDelete).not.toHaveBeenCalled();
});

it("clears drafts before confirmed deletion and respects an active-session lock", async () => {
  await render({ ...props, profile: { ...profile, credentialState: "missing" } });
  await change("input", "synthetic-asr");
  await render({ ...props, confirmingDelete: true });
  const buttons = [...host.querySelectorAll("button")];
  await act(async () => buttons.find((button) => button.textContent === I18N.settings.confirmDelete)!.click());
  expect(props.onConfirmDelete).toHaveBeenCalledOnce();
  expect((host.querySelector("input") as HTMLInputElement).value).toBe("");
  await render({ ...props, disabled: true });
  expect([...host.querySelectorAll<HTMLInputElement>("input")].every((node) => node.disabled)).toBe(true);
  expect(picker().disabled).toBe(true);
});

it("uses the unified picker with keyboard selection without submitting the form", async () => {
  await render();
  expect(picker().getAttribute("aria-label")).toBe(I18N.settings.textTranslationLabel);
  await key("Enter"); await key("End"); await key("Enter");
  expect(picker().textContent).toBe(I18N.settings.textTranslationOpenAICompatible);
  expect(document.activeElement).toBe(picker());
  expect(props.onSave).not.toHaveBeenCalled();
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
});

it("follows externally updated saved destinations while the picker is open", async () => {
  await render();
  await act(() => picker().click());
  await render({ ...props, profile: { ...profile, textTranslation: "deepLX" } });
  expect(picker().textContent).toBe(I18N.settings.textTranslationCustom);
  expect(document.querySelector('[role="option"][aria-selected="true"]')?.textContent).toBe(I18N.settings.textTranslationCustom);
  await key("Enter");
  expect(picker().textContent).toBe(I18N.settings.textTranslationCustom);
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(props.onSave).not.toHaveBeenCalled();
});

it("configures the official DeepL destination with a key and no address field", async () => {
  await render(); await chooseTranslation("deepL");
  expect(picker().textContent).toBe("DeepL");
  expect(host.querySelector("#test-endpoint")).toBeNull();
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
  expect(host.textContent).toContain(I18N.settings.deepLApiKey);
  await change("#test-token", " synthetic-deepl-key "); await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "alibabaTranslation", model: "", apiKey: "", textTranslation: "deepL", endpoint: "", token: "synthetic-deepl-key" });
});

it("clears destination secrets when switching between DeepL and a custom service", async () => {
  await render(); await chooseTranslation("deepL");
  await change("#test-token", "synthetic-deepl-key");
  await chooseTranslation("deepLX");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  await change("#test-endpoint", "https://example.com/translate");
  await change("#test-token", "synthetic-custom-token");
  await chooseTranslation("deepL");
  expect(host.querySelector("#test-endpoint")).toBeNull();
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
  expect(props.onSave).not.toHaveBeenCalled();
});

it("keeps a saved DeepL key hidden by default and permits replacing only the Alibaba key", async () => {
  await render({ ...props, profile: { ...profile, textTranslation: "deepL" } });
  expect(host.querySelector("#test-endpoint")).toBeNull();
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.placeholder).toBe(I18N.settings.savedTranslationKeyPlaceholder);
  const replace = [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.replaceCredentials)!;
  await act(() => replace.click()); await change("#test-apiKey", "synthetic-new-asr"); await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "alibabaTranslation", model: "", apiKey: "synthetic-new-asr", textTranslation: "deepL", endpoint: "", token: "" });
});

it("reveals a saved translation key only on demand and clears it on blur or a hidden editor", async () => {
  await render({ ...props, profile: { ...profile, textTranslation: "deepL" } });
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(host.querySelector(".service-stage--translation .stored-credential-reveal button")).not.toBeNull();
  expect(host.querySelector(".stored-credential-reveal input")).toBeNull();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-saved-deepl-key");
  await act(async () => { host.querySelector<HTMLButtonElement>(".stored-credential-reveal button")!.click(); });
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: profile.id, field: "token", textTranslation: "deepL" });
  expect(host.querySelector<HTMLInputElement>(".stored-credential-reveal input")!.value).toBe("synthetic-saved-deepl-key");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(props.onSave).not.toHaveBeenCalled();
  await act(() => window.dispatchEvent(new Event("blur")));
  expect(host.querySelector(".stored-credential-reveal input")).toBeNull();
  await act(async () => { host.querySelector<HTMLButtonElement>(".stored-credential-reveal button")!.click(); });
  expect(host.querySelector<HTMLInputElement>(".stored-credential-reveal input")!.value).toBe("synthetic-saved-deepl-key");
  await render({ ...props, visible: false });
  expect(host.querySelector(".stored-credential-reveal")).toBeNull();
  await render({ ...props, visible: true });
  expect(host.querySelector(".stored-credential-reveal input")).toBeNull();
});

it("discards an in-flight saved DeepL reveal when the draft selects another route", async () => {
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  await render({ ...props, profile: { ...profile, textTranslation: "deepL" } });
  await act(() => host.querySelector<HTMLButtonElement>(".stored-credential-reveal button")!.click());
  await chooseTranslation("deepLX");
  await act(async () => { complete("synthetic-old-route-key"); });
  expect(host.querySelector(".stored-credential-reveal")).toBeNull();
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  await chooseTranslation("deepL");
  expect(host.querySelector(".stored-credential-reveal input")).toBeNull();
  expect(profileRevealCredential).toHaveBeenCalledOnce();
});

it("requests the legacy ASR field when viewing a historical custom profile's saved recognition key", async () => {
  await render({ ...props, profile: { ...profile, provider: "deepLX" } });
  await act(() => [...host.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent === I18N.settings.replaceCredentials)!.click());
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-asr-key");
  await act(async () => { host.querySelector<HTMLButtonElement>(".stored-credential-reveal button")!.click(); });
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: profile.id, field: "asrApiKey" });
  expect(host.querySelector<HTMLInputElement>("#test-apiKey")!.value).toBe("");
});

it("does not carry a DeepL key draft into an externally selected custom destination", async () => {
  await render({ ...props, profile: { ...profile, textTranslation: "deepL" } });
  await change("#test-token", "synthetic-deepl-key");
  await render({ ...props, profile: { ...profile, textTranslation: "deepLX" } });
  expect(picker().textContent).toBe(I18N.settings.textTranslationCustom);
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("");
  expect(props.onSave).not.toHaveBeenCalled();
});

it("uses platform-aware Linux guidance for unavailable storage in each language", async () => {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla Linux");
  for (const language of ["en", "zh", "ja"] as const) {
    setStoredUiLanguage(language);
    await render({ ...props, profile: { ...profile, credentialState: "unavailable" } });
    expect(host.querySelector('[role="status"]')?.textContent).toBe(diagnosticCopy("linux").storage);
    expect(host.querySelector('[role="status"]')?.textContent).toContain("GNOME Keyring");
  }
});

it("shows one actionable credential error after a failed save", async () => {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla Linux");
  await render({ ...props, profile: { ...profile, credentialState: "unavailable" }, feedback: { tone: "error", message: diagnosticCopy("linux").serviceUnavailable } });
  expect(host.textContent).toContain(diagnosticCopy("linux").serviceUnavailable);
  expect(host.textContent).not.toContain(diagnosticCopy("linux").storage);
  await render({ ...props, feedback: { tone: "error", message: I18N.settings.profileActionFailed } });
  expect(host.textContent).toContain(diagnosticCopy("linux").storage);
});

it("discards the write-only draft after a successful save", async () => {
  await render({ ...props, profile: { ...profile, credentialState: "unavailable" }, onSave: vi.fn().mockResolvedValue({}) });
  await change("input", "synthetic-asr"); await submit();
  expect((host.querySelector("input") as HTMLInputElement).value).toBe("");
});

it("adds an OpenAI-compatible destination without repeating the saved recognition key", async () => {
  await render(); await chooseTranslation("openAICompatible");
  expect(host.textContent).toContain(I18N.settings.openAICompatibleRequirements);
  expect(host.textContent).toContain(I18N.settings.openAICompatibleLanguages);
  expect(host.textContent).toContain(I18N.settings.openAICompatibleRequired);
  expect(host.querySelectorAll(".service-stage p")).toHaveLength(0);
  expect(host.querySelector(".service-stage--translation .settings-help-control__description")?.textContent).toContain(I18N.settings.openAICompatibleRequirements);
  expect(host.querySelector('.service-stage:not(.service-stage--translation) .provider-icon[data-provider="alibabaCloud"]')).not.toBeNull();
  expect(picker().querySelector('.provider-icon[data-provider="openAICompatible"]')).not.toBeNull();
  expect(host.querySelector("#test-apiKey")).toBeNull();
  const endpoint = host.querySelector<HTMLInputElement>("#test-endpoint")!;
  const model = host.querySelector<HTMLInputElement>("#test-model")!;
  const token = host.querySelector<HTMLInputElement>("#test-token")!;
  expect(endpoint.value).toBe(""); expect(model.value).toBe(""); expect(token.value).toBe("");
  expect(endpoint.placeholder).toBe("https://dashscope.aliyuncs.com/compatible-mode/v1");
  expect(model.placeholder).toBe("qwen-turbo");
  expect([endpoint, model, token].every(node => node.required)).toBe(true);
  await change("#test-endpoint", "https://dashscope.aliyuncs.com/compatible-mode/v1");
  await change("#test-token", " synthetic-translation-key ");
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
  await change("#test-model", " qwen-turbo "); await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1", token: "synthetic-translation-key", model: "qwen-turbo" });
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("keeps saved OpenAI-compatible fields write-only and permits changing only the model", async () => {
  await render({ ...props, profile: { ...profile, textTranslation: "openAICompatible" } });
  expect(host.querySelector(".stored-credential-reveal")).toBeNull();
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(host.querySelector<HTMLInputElement>("#test-model")!.placeholder).toBe(I18N.settings.savedTranslationModelPlaceholder);
  await change("#test-model", "new-model"); await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "", token: "", model: "new-model" });
  await change("#test-endpoint", "https://new.example/v1");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.required).toBe(true);
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
  expect(host.textContent).toContain(I18N.settings.openAICompatibleAddressKey);
});

it("focuses an unsafe OpenAI-compatible address before saving any key", async () => {
  await render(); await chooseTranslation("openAICompatible");
  await change("#test-endpoint", "https://example.com/v1?key=synthetic-key");
  await change("#test-model", "model"); await change("#test-token", "synthetic-key"); await submit();
  expect(props.onSave).not.toHaveBeenCalled();
  expect(document.activeElement?.id).toBe("test-endpoint");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.deepLXEndpointInvalid);
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("synthetic-key");
});

it("focuses an invalid model and clears all destination drafts when changing routes", async () => {
  await render(); await chooseTranslation("openAICompatible");
  await change("#test-endpoint", "https://example.com/v1/chat/completions");
  await change("#test-model", "x".repeat(257)); await change("#test-token", "synthetic-key"); await submit();
  expect(props.onSave).not.toHaveBeenCalled();
  expect(document.activeElement?.id).toBe("test-model");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.openAICompatibleModelInvalid);
  await chooseTranslation("deepLX");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(host.querySelector("#test-model")).toBeNull();
  await chooseTranslation("openAICompatible");
  expect(host.querySelector<HTMLInputElement>("#test-model")!.value).toBe("");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("retains third-party drafts after a storage error and clears them after successful saving", async () => {
  await render(); await chooseTranslation("openAICompatible");
  await change("#test-endpoint", "https://synthetic.example/v1");
  await change("#test-model", "synthetic-model"); await change("#test-token", "synthetic-translation-key"); await submit();
  await render({ ...props, feedback: { tone: "error", message: diagnosticCopy("linux").storage } });
  expect(host.textContent).toContain(diagnosticCopy("linux").storage);
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("synthetic-translation-key");
  expect(host.querySelector<HTMLInputElement>("#test-model")!.value).toBe("synthetic-model");
  expect(profileRevealCredential).not.toHaveBeenCalled();
  await render({ ...props, profile: { ...profile, textTranslation: "openAICompatible" }, feedback: null, onSave: vi.fn().mockResolvedValue({}) });
  await submit();
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(host.querySelector<HTMLInputElement>("#test-model")!.value).toBe("");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("");
});
