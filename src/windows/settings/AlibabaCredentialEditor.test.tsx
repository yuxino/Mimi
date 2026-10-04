// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { diagnosticCopy } from "../../lib/connectionDiagnostics";
import { profileCredentialEditorState, profileRevealCredential } from "../../lib/ipc";
import type { ServiceProfile, TextTranslation } from "../../lib/types";
import { AlibabaCredentialEditor } from "./AlibabaCredentialEditor";

vi.mock("../../lib/ipc", () => ({ isTauri: false, profileRevealCredential: vi.fn(), profileCredentialEditorState: vi.fn(), setOverlayPointerCursor: vi.fn() }));

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
  vi.mocked(profileCredentialEditorState).mockReset().mockImplementation(async ({ textTranslation }) => ({ savedFields: textTranslation ? ["token"] : ["apiKey", "asrApiKey"] }));
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  props = { profile, inputId: "test", disabled: false, busy: false, feedback: null, onSave: vi.fn().mockResolvedValue(null), onRequestDelete: vi.fn(), onConfirmDelete: vi.fn(), confirmingDelete: false, onCancelDelete: vi.fn() };
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); setStoredUiLanguage("en"); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function render(next = props) { props = next; await act(async () => { root.render(<AlibabaCredentialEditor {...props} />); }); }
async function change(selector: string, value: string) {
  const node = host.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector)!;
  await act(() => {
    Object.getOwnPropertyDescriptor(node instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(node, value);
    node.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
function picker() { return host.querySelector<HTMLButtonElement>('.service-stage--translation [role="combobox"]')!; }
async function chooseTranslation(value: TextTranslation) {
  await act(() => picker().click());
  const label = value === "chatMock" ? "ChatMock" : value === "openAICompatible" ? I18N.settings.textTranslationOpenAICompatible : value === "deepLX" ? I18N.settings.textTranslationCustom : value === "deepL" ? "DeepL" : I18N.settings.textTranslationFollow;
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === label)!;
  await act(async () => { option.click(); });
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
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.placeholder).not.toMatch(/留空|Leave blank|空欄/);
  await chooseTranslation("followService"); await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "alibabaTranslation", model: "", apiKey: "", textTranslation: "followService", endpoint: "", token: "" });
});

it.each(["en", "zh", "ja"] as const)("shows both stages in read-only file mode without exposing any credential editor in %s", async (language) => {
  setStoredUiLanguage(language);
  await render({ ...props, readOnly: true, profile: { ...profile, textTranslation: "openAICompatible" } });
  expect([...host.querySelectorAll(".service-stage h3")].map(node => node.textContent)).toEqual([I18N.settings.speechRecognition, I18N.settings.textTranslationLabel]);
  expect(host.querySelector('[role="combobox"]')).toBeNull();
  expect(host.querySelector('.service-stage--translation .service-stage__provider')?.textContent).toBe(I18N.settings.textTranslationOpenAICompatible);
  expect(host.querySelector('.service-stage__restriction [role="status"]')?.textContent).toBe(diagnosticCopy().localDevTranslationLocked);
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
  const buttons = [...document.querySelectorAll("button")];
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
  expect(host.querySelector<HTMLInputElement>("#test-token")!.placeholder).toBe("••••••••");
  const replace = [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.replaceCredentials)!;
  await act(() => replace.click()); await change("#test-apiKey", "synthetic-new-asr"); await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "alibabaTranslation", model: "", apiKey: "synthetic-new-asr", textTranslation: "deepL", endpoint: "", token: "" });
});

it("reveals and edits a saved translation key in its original input and clears loaded values on blur or hide", async () => {
  await render({ ...props, profile: { ...profile, textTranslation: "deepL" } });
  expect(profileRevealCredential).not.toHaveBeenCalled();
  const input = host.querySelector<HTMLInputElement>("#test-token")!;
  const toggle = () => host.querySelector<HTMLButtonElement>(".service-stage--translation .saved-credential-input__toggle")!;
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-saved-deepl-key");
  await act(async () => { toggle().click(); });
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: profile.id, field: "token", textTranslation: "deepL" });
  expect(host.querySelectorAll("#test-token")).toHaveLength(1);
  expect(input.value).toBe("synthetic-saved-deepl-key");
  expect(input.type).toBe("text");
  expect(host.querySelector('button[type="submit"]')).toBeNull();
  await act(() => window.dispatchEvent(new Event("blur")));
  expect(input.value).toBe("");
  expect(input.type).toBe("password");
  await act(async () => { toggle().click(); });
  await render({ ...props, visible: false });
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(host.querySelector(".saved-credential-input__toggle")).toBeNull();
  await render({ ...props, visible: true });
  await act(async () => { toggle().click(); });
  await change("#test-token", "synthetic-edited-key");
  await submit();
  expect(props.onSave).toHaveBeenCalledWith(expect.objectContaining({ token: "synthetic-edited-key" }));
});

it("discards an in-flight saved DeepL reveal when the draft selects another route", async () => {
  let complete!: (value: string | null) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  await render({ ...props, profile: { ...profile, textTranslation: "deepL" } });
  await act(() => host.querySelector<HTMLButtonElement>(".service-stage--translation .saved-credential-input__toggle")!.click());
  await chooseTranslation("deepLX");
  await act(async () => { complete("synthetic-old-route-key"); });
  expect(host.querySelector(".service-stage--translation .saved-credential-input__toggle")).toBeNull();
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  await chooseTranslation("deepL");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(profileRevealCredential).toHaveBeenCalledOnce();
});

it("requests the legacy ASR field in the original recognition input", async () => {
  await render({ ...props, profile: { ...profile, provider: "deepLX" } });
  await act(() => [...host.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent === I18N.settings.replaceCredentials)!.click());
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-asr-key");
  await act(async () => { host.querySelector<HTMLButtonElement>(".saved-credential-input__toggle")!.click(); });
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: profile.id, field: "asrApiKey" });
  expect(host.querySelector<HTMLInputElement>("#test-apiKey")!.value).toBe("synthetic-asr-key");
  expect(props.onSave).not.toHaveBeenCalled();
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

it("uses local file guidance for unavailable storage in each language", async () => {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla Linux");
  for (const language of ["en", "zh", "ja"] as const) {
    setStoredUiLanguage(language);
    await render({ ...props, profile: { ...profile, credentialState: "unavailable" } });
    expect(host.querySelector('[role="status"]')?.textContent).toBe(diagnosticCopy("linux").storage);
    expect(host.querySelector('[role="status"]')?.textContent).not.toMatch(/GNOME Keyring|Secret Service|钥匙串|キーチェーン/);
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
  expect(endpoint.placeholder).toBe(I18N.settings.serviceAddressPlaceholder);
  expect(model.placeholder).toBe(I18N.settings.modelNamePlaceholder);
  expect([endpoint, model].every(node => node.required)).toBe(true);
  expect(token.required).toBe(false);
  expect(token.placeholder).toBe(I18N.settings.optionalTranslationKeyPlaceholder);
  await change("#test-endpoint", "https://dashscope.aliyuncs.com/compatible-mode/v1");
  await change("#test-token", " synthetic-translation-key ");
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
  await change("#test-model", " qwen-turbo "); await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1", token: "synthetic-translation-key", model: "qwen-turbo" });
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("keeps saved OpenAI-compatible values unread until revealed and permits changing only the model", async () => {
  await render({ ...props, profile: { ...profile, textTranslation: "openAICompatible" } });
  expect(host.querySelectorAll(".saved-credential-input__toggle")).toHaveLength(1);
  expect(host.querySelector(".stored-credential-reveal input")).toBeNull();
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(host.querySelector<HTMLInputElement>("#test-model")!.placeholder).toBe(I18N.settings.modelNamePlaceholder);
  await change("#test-model", "new-model"); await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "", token: "", model: "new-model" });
  await change("#test-endpoint", "https://new.example/v1");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.required).toBe(false);
  expect(host.querySelector<HTMLInputElement>("#test-token")!.placeholder).toBe(I18N.settings.optionalTranslationKeyPlaceholder);
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(false);
  expect(host.textContent).toContain(I18N.settings.openAICompatibleAddressKey);
  await submit();
  expect(props.onSave).toHaveBeenLastCalledWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "https://new.example/v1", token: "", model: "new-model" });
});

it("saves a keyless ChatMock destination while keeping its requirements in help", async () => {
  await render(); await chooseTranslation("chatMock");
  expect(picker().textContent).toBe("ChatMock");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("http://127.0.0.1:8000/v1");
  expect(host.querySelector<HTMLInputElement>("#test-model")!.value).toBe("");
  expect(host.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true);
  const help = host.querySelector(".service-stage--translation .settings-help-control__description")!;
  expect(help.textContent).toContain(I18N.settings.chatMockSetup);
  expect(host.querySelectorAll(".service-stage p")).toHaveLength(0);
  await change("#test-endpoint", "http://localhost:8080/v1");
  await change("#test-model", "synthetic-model");
  expect(host.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(false);
  await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "chatMock", endpoint: "http://localhost:8080/v1", token: "", model: "synthetic-model" });
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("still requires an Alibaba recognition key when the translation service needs no key", async () => {
  await render({ ...props, profile: { ...profile, credentialState: "missing" } });
  await chooseTranslation("openAICompatible");
  await change("#test-endpoint", "http://localhost:8080/v1");
  await change("#test-model", "synthetic-model");
  expect(host.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true);
  await submit();
  expect(props.onSave).not.toHaveBeenCalled();
  await change("#test-apiKey", "synthetic-asr");
  await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "synthetic-asr", textTranslation: "openAICompatible", endpoint: "http://localhost:8080/v1", token: "", model: "synthetic-model" });
});

it.each(["customDashScopeASR", "customOpenAIASR"] as const)("saves keyless ChatMock independently from %s recognition", async (provider) => {
  await render({ ...props, profile: { ...profile, provider, credentialState: "missing", speechCredentialState: "present", textCredentialState: "missing" }, textOnly: true });
  await chooseTranslation("chatMock");
  expect(host.querySelector("#test-apiKey")).toBeNull();
  expect(host.querySelector(".service-stage--translation .settings-help-control__description")!.textContent).toContain(I18N.settings.chatMockSetup);
  await change("#test-endpoint", "http://127.0.0.1:8080/v1");
  await change("#test-model", "synthetic-model");
  await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "chatMock", endpoint: "http://127.0.0.1:8080/v1", token: "", model: "synthetic-model" });
});

it("keeps removal of a saved translation key reversible and separate from empty saved fields", async () => {
  const textConnectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...profile, textTranslation: "openAICompatible" }, textConnectionCheck });
  const action = (label: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === label)!;
  expect(textConnectionCheck).toHaveBeenLastCalledWith(undefined);
  await act(() => action(I18N.settings.removeTranslationApiKey).click());
  expect(props.onSave).not.toHaveBeenCalled();
  expect(host.querySelector<HTMLInputElement>("#test-token")!.disabled).toBe(true);
  expect(host.querySelector<HTMLInputElement>("#test-token")!.placeholder).toBe(I18N.settings.noTranslationKeyPlaceholder);
  expect(textConnectionCheck).toHaveBeenLastCalledWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "", token: "", model: "", clearToken: true });
  await act(() => action(I18N.settings.cancelTranslationKeyRemoval).click());
  expect(host.querySelector<HTMLInputElement>("#test-token")!.disabled).toBe(false);
  expect(host.querySelector<HTMLInputElement>("#test-token")!.placeholder).toBe("••••••••");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(undefined);
  expect(host.querySelector('button[type="submit"]')).toBeNull();
  await act(() => action(I18N.settings.removeTranslationApiKey).click());
  await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "", token: "", model: "", clearToken: true });
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("discards pending key removal when cancelling or switching text destinations", async () => {
  await render({ ...props, profile: { ...profile, textTranslation: "openAICompatible" } });
  const action = (label: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === label)!;
  await act(() => action(I18N.settings.removeTranslationApiKey).click());
  await act(async () => { action(I18N.settings.cancel).click(); });
  expect(host.querySelector<HTMLInputElement>("#test-token")!.disabled).toBe(false);
  expect(host.querySelector('button[type="submit"]')).toBeNull();
  await act(() => action(I18N.settings.removeTranslationApiKey).click());
  await chooseTranslation("deepL");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.disabled).toBe(false);
  await change("#test-token", "synthetic-deepl");
  await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "deepL", endpoint: "", token: "synthetic-deepl", model: "" });
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


it("keeps generic and ChatMock drafts separate and only presets ChatMock", async () => {
  await render(); await chooseTranslation("openAICompatible");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("");
  expect(host.querySelector(".service-stage--translation .settings-help-control__description")!.textContent).not.toContain(I18N.settings.chatMockSetup);
  await change("#test-endpoint", "https://synthetic.example/v1");
  await change("#test-token", "synthetic-other-provider-key");
  await change("#test-model", "other-model");
  await chooseTranslation("chatMock");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("http://127.0.0.1:8000/v1");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(host.querySelector<HTMLInputElement>("#test-model")!.value).toBe("");
  expect(props.onSave).not.toHaveBeenCalled();
  await change("#test-model", "synthetic-model"); await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "chatMock", endpoint: "http://127.0.0.1:8000/v1", token: "", model: "synthetic-model" });
  await chooseTranslation("openAICompatible");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("");
  expect(host.querySelector<HTMLInputElement>("#test-model")!.value).toBe("");
});

it("never replaces a saved ChatMock destination with the local preset on reopen", async () => {
  await render({ ...props, profile: { ...profile, textTranslation: "chatMock" } });
  expect(picker().textContent).toBe("ChatMock");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("");
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.placeholder).not.toMatch(/留空|Leave blank|空欄/);
  expect(host.querySelector('button[type="submit"]')).toBeNull();
  await change("#test-model", "new-model"); await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "chatMock", endpoint: "", token: "", model: "new-model" });
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it.each(["openAICompatible", "chatMock"] as const)("reveals each saved %s key in its original field", async route => {
  await render({ ...props, profile: { ...profile, textTranslation: route } });
  await act(() => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.replaceCredentials)!.click());
  expect(profileRevealCredential).not.toHaveBeenCalled();
  vi.mocked(profileRevealCredential).mockResolvedValueOnce("synthetic-recognition-key").mockResolvedValueOnce("synthetic-translation-key");
  await act(async () => host.querySelector<HTMLButtonElement>('.service-stage:not(.service-stage--translation) .saved-credential-input__toggle')!.click());
  expect(profileRevealCredential).toHaveBeenLastCalledWith({ profileId: profile.id, field: "apiKey" });
  expect(host.querySelector<HTMLInputElement>("#test-apiKey")!.value).toBe("synthetic-recognition-key");
  await act(async () => host.querySelector<HTMLButtonElement>('.service-stage--translation .saved-credential-input__toggle')!.click());
  expect(profileRevealCredential).toHaveBeenLastCalledWith({ profileId: profile.id, field: "token", textTranslation: route });
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("synthetic-translation-key");
  expect(host.querySelector('input[readonly]')).toBeNull();
  expect(props.onSave).not.toHaveBeenCalled();
});

it.each(["openAICompatible", "chatMock"] as const)("shows saved %s configuration with no false optional-key action", async route => {
  vi.mocked(profileCredentialEditorState).mockImplementation(async ({ textTranslation }) => textTranslation
    ? { savedFields: [], endpoint: "http://127.0.0.1:8080/v1", model: "synthetic-model" }
    : { savedFields: ["apiKey"] });
  const textConnectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...profile, textTranslation: route }, textConnectionCheck });
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("http://127.0.0.1:8080/v1");
  expect(host.querySelector<HTMLInputElement>("#test-model")!.value).toBe("synthetic-model");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.value).toBe("");
  expect(host.querySelector<HTMLInputElement>("#test-token")!.placeholder).toBe(I18N.settings.optionalTranslationKeyPlaceholder);
  expect(host.querySelector(".service-stage--translation .saved-credential-input__toggle")).toBeNull();
  expect([...host.querySelectorAll("button")].some(button => button.textContent === I18N.settings.removeTranslationApiKey)).toBe(false);
  expect(host.textContent).not.toContain(I18N.settings.savedCredentialMissing);
  expect(textConnectionCheck).toHaveBeenLastCalledWith(undefined);
  expect(host.querySelector('button[type="submit"]')).toBeNull();
  await change("#test-model", "new-model"); await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: route, endpoint: "", token: "", model: "new-model" });
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it.each(["openAICompatible", "chatMock"] as const)("expands long saved %s address and model into the same editable draft", async route => {
  const endpoint = `https://synthetic.example/${"translation-path/".repeat(16)}v1`;
  const model = `synthetic-${"model-".repeat(24)}revision`;
  vi.mocked(profileCredentialEditorState).mockImplementation(async ({ textTranslation }) => textTranslation
    ? { savedFields: [], endpoint, model } : { savedFields: ["apiKey"] });
  await render({ ...props, profile: { ...profile, textTranslation: route } });
  const endpointGroup = host.querySelector("#test-endpoint")!.closest(".config-input-group")!;
  await act(() => endpointGroup.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
  expect(endpointGroup.querySelector("input")).toBeNull();
  expect(endpointGroup.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe(endpoint);
  expect(endpointGroup.querySelector<HTMLTextAreaElement>("textarea")!.readOnly).toBe(false);
  expect(endpointGroup.querySelector<HTMLTextAreaElement>("textarea")!.wrap).toBe("soft");
  expect(props.onSave).not.toHaveBeenCalled();
  const editedEndpoint = `${endpoint}/edited`;
  await change("#test-endpoint", editedEndpoint);
  await act(() => endpointGroup.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe(editedEndpoint);

  const modelGroup = host.querySelector("#test-model")!.closest(".config-input-group")!;
  await act(() => modelGroup.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
  expect(modelGroup.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe(model);
  const editedModel = `${model}-edited`;
  await change("#test-model", editedModel);
  expect(props.onSave).not.toHaveBeenCalled();
  await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: route,
    endpoint: editedEndpoint, token: "", model: editedModel });
  expect(modelGroup.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe(editedModel);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(host.querySelector("#test-token")!.closest(".config-input-group")!.querySelector(".config-input__expand")).toBeNull();
});

it("does not overwrite edits when saved configuration arrives late", async () => {
  let complete!: (value: Awaited<ReturnType<typeof profileCredentialEditorState>>) => void;
  vi.mocked(profileCredentialEditorState).mockImplementation(({ textTranslation }) => textTranslation
    ? new Promise(resolve => { complete = resolve; }) : Promise.resolve({ savedFields: ["apiKey"] }));
  await render({ ...props, profile: { ...profile, textTranslation: "openAICompatible" } });
  await change("#test-endpoint", "https://edited.example/v1");
  await act(async () => complete({ savedFields: [], endpoint: "http://127.0.0.1:8080/v1", model: "synthetic-model" }));
  expect(host.querySelector<HTMLInputElement>("#test-endpoint")!.value).toBe("https://edited.example/v1");
  expect(host.querySelector<HTMLInputElement>("#test-model")!.value).toBe("synthetic-model");
  await change("#test-endpoint", ""); await submit();
  expect(props.onSave).not.toHaveBeenCalled();
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.deepLXEndpointInvalid);
});


it("waits for saved destination metadata before a save can clear or reuse its token", async () => {
  let complete!: (value: Awaited<ReturnType<typeof profileCredentialEditorState>>) => void;
  const textConnectionCheck = vi.fn().mockReturnValue(null);
  vi.mocked(profileCredentialEditorState).mockImplementation(({ textTranslation }) => textTranslation
    ? new Promise(resolve => { complete = resolve; }) : Promise.resolve({ savedFields: ["apiKey"] }));
  await render({ ...props, profile: { ...profile, textTranslation: "deepLX" }, textConnectionCheck });
  await change("#test-endpoint", "https://example.com/translate");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-token", "synthetic-new-token");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(expect.objectContaining({ endpoint: "https://example.com/translate", token: "synthetic-new-token" }));
  expect(host.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true);
  await submit();
  expect(props.onSave).not.toHaveBeenCalled();
  await act(async () => complete({ savedFields: ["token"], endpoint: "https://example.com/translate" }));
  expect(textConnectionCheck).toHaveBeenLastCalledWith(expect.objectContaining({ endpoint: "https://example.com/translate", token: "synthetic-new-token" }));
  await submit();
  expect(props.onSave).toHaveBeenCalledWith(expect.objectContaining({ endpoint: "", token: "synthetic-new-token" }));
});

it("checks a complete translation draft before saving or configuring recognition", async () => {
  const textConnectionCheck = vi.fn().mockReturnValue(null);
  const connectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...profile, credentialState: "missing", speechCredentialState: "missing", textCredentialState: "missing" }, textConnectionCheck, connectionCheck });
  await chooseTranslation("openAICompatible");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-endpoint", "https://translation.example/v1");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-model", " synthetic-model ");
  expect(textConnectionCheck).toHaveBeenLastCalledWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "https://translation.example/v1", model: "synthetic-model", token: "" });
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  expect(props.onSave).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
  await change("#test-apiKey", "synthetic-speech-key");
  expect(connectionCheck).toHaveBeenLastCalledWith({ kind: "alibabaTranslation", apiKey: "synthetic-speech-key", textTranslation: "followService", endpoint: "", model: "", token: "" });
  expect(textConnectionCheck).toHaveBeenLastCalledWith(expect.objectContaining({ apiKey: "", textTranslation: "openAICompatible" }));
  await change("#test-token", "synthetic-text-key");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(expect.objectContaining({ apiKey: "", token: "synthetic-text-key" }));
  expect(props.onSave).not.toHaveBeenCalled();
});

it("allows retrying unavailable saved recognition without requiring a replacement key", async () => {
  vi.mocked(profileCredentialEditorState).mockRejectedValue("credential_store_unavailable");
  const connectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...profile, credentialState: "unavailable" }, connectionCheck });
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined);
  await change("#test-apiKey", " ");
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-apiKey", "synthetic-replacement-key");
  expect(connectionCheck).toHaveBeenLastCalledWith(expect.objectContaining({ apiKey: "synthetic-replacement-key" }));
  await change("#test-apiKey", "");
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined);
  expect(props.onSave).not.toHaveBeenCalled();
});

it("checks an explicit complete text replacement when saved metadata cannot be read", async () => {
  vi.mocked(profileCredentialEditorState).mockRejectedValue("credential_store_unavailable");
  const textConnectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...profile, textTranslation: "openAICompatible" }, textConnectionCheck });
  await change("#test-endpoint", "https://replacement.example/v1");
  await change("#test-model", "replacement-model");
  // Blank optional auth could still mean retain a saved token; do not guess.
  expect(textConnectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-token", "synthetic-explicit-key");
  expect(textConnectionCheck).toHaveBeenLastCalledWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "https://replacement.example/v1", model: "replacement-model", token: "synthetic-explicit-key" });
  expect(props.onSave).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("checks edited saved translation values with visible configuration and native auth reuse", async () => {
  vi.mocked(profileCredentialEditorState).mockImplementation(async ({ textTranslation }) => textTranslation
    ? { savedFields: ["token"], endpoint: "https://saved.example/v1/chat/completions", model: "saved-model" }
    : { savedFields: ["apiKey"] });
  const textConnectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...profile, textTranslation: "openAICompatible" }, textConnectionCheck });
  expect(textConnectionCheck).toHaveBeenLastCalledWith(undefined);
  await change("#test-model", "edited-model");
  expect(textConnectionCheck).toHaveBeenLastCalledWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "https://saved.example/v1/chat/completions", token: "", model: "edited-model" });
  await change("#test-endpoint", "https://new.example/v1");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(expect.objectContaining({ endpoint: "https://new.example/v1", token: "", model: "edited-model" }));
  await change("#test-endpoint", "https://new.example/v1?key=synthetic");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-endpoint", "https://saved.example/v1/chat/completions");
  await change("#test-model", "x".repeat(257));
  expect(textConnectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-model", "saved-model");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(undefined);
  expect(props.onSave).not.toHaveBeenCalled();
});

it("checks a legacy DeepLX token edit using the visible saved endpoint without a speech credential draft", async () => {
  vi.mocked(profileCredentialEditorState).mockImplementation(async ({ textTranslation }) => textTranslation
    ? { savedFields: ["token"], endpoint: "https://legacy.example/translate" }
    : { savedFields: ["asrApiKey"] });
  const textConnectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...profile, provider: "deepLX" }, textConnectionCheck });
  await change("#test-token", "synthetic-new-token");
  expect(textConnectionCheck).toHaveBeenLastCalledWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "deepLX", endpoint: "https://legacy.example/translate", token: "synthetic-new-token", model: "" });
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(props.onSave).not.toHaveBeenCalled();
  await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "deepLX", endpoint: "", token: "synthetic-new-token", model: "" });
});

it("checks DeepL and built-in translation drafts using only the relevant key", async () => {
  const textConnectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...profile, credentialState: "missing" }, textConnectionCheck });
  await change("#test-apiKey", "synthetic-speech-key");
  expect(textConnectionCheck).toHaveBeenLastCalledWith({ kind: "alibabaTranslation", apiKey: "synthetic-speech-key", textTranslation: "followService", endpoint: "", token: "", model: "" });
  await chooseTranslation("deepL");
  expect(textConnectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-token", "synthetic-deepl-key");
  expect(textConnectionCheck).toHaveBeenLastCalledWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "deepL", endpoint: "", token: "synthetic-deepl-key", model: "" });
  await render({ ...props, readOnly: true });
  expect(textConnectionCheck).toHaveBeenLastCalledWith(undefined);
  expect(props.onSave).not.toHaveBeenCalled();
});
