// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { profileCredentialEditorState, profileRevealCredential } from "../../lib/ipc";
import { CustomSpeechCredentialEditor } from "./CustomSpeechCredentialEditor";

vi.mock("../../lib/ipc", () => ({ isTauri: false, profileRevealCredential: vi.fn(), profileCredentialEditorState: vi.fn(), setOverlayPointerCursor: vi.fn() }));
let root: Root, host: HTMLDivElement;
let props: Parameters<typeof CustomSpeechCredentialEditor>[0];
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  Element.prototype.scrollIntoView = vi.fn();
  vi.mocked(profileCredentialEditorState).mockReset().mockResolvedValue({ savedFields: ["apiKey"] });
  vi.mocked(profileRevealCredential).mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  props = { profile: { id: "custom", name: "Custom", provider: "customDashScopeASR", credentialState: "missing", speechCredentialState: "missing", textCredentialState: "present" }, inputId: "test", disabled: false, busy: false, feedback: null, onSave: vi.fn().mockResolvedValue(null), onRequestDelete: vi.fn(), onConfirmDelete: vi.fn(), confirmingDelete: false, onCancelDelete: vi.fn() };
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("en"); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function render(next = props) { props = next; await act(async () => root.render(<CustomSpeechCredentialEditor {...props} />)); }
async function change(selector: string, value: string) {
  const input = host.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector)!;
  await act(async () => { Object.getOwnPropertyDescriptor(input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
}
async function submit(selector = ".service-stage:not(.service-stage--translation) form") { await act(async () => { const node = host.querySelector(selector)!; (node instanceof HTMLFormElement ? node : node.closest("form")!).dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); }); }
async function fillSpeech(endpoint = "wss://speech.example/inference") {
  await change("#test-speech-endpoint", endpoint); await change("#test-speech-model", "synthetic-asr-model"); await change("#test-speech-key", "synthetic-asr-key");
}
it("checks a complete speech draft without saving and disables incomplete or unsafe drafts", async () => {
  const connectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, connectionCheck });
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-speech-endpoint", "wss://speech.example/inference");
  await change("#test-speech-model", "synthetic-asr-model");
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-speech-key", "synthetic-asr-key");
  expect(connectionCheck).toHaveBeenLastCalledWith({ kind: "customSpeech", endpoint: "wss://speech.example/inference", model: "synthetic-asr-model", apiKey: "synthetic-asr-key" });
  await change("#test-speech-endpoint", "wss://speech.example?key=synthetic");
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-speech-endpoint", "wss://speech.example/inference");
  await change("#test-speech-model", "x".repeat(257));
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  expect(props.onSave).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it.each(["unavailable", undefined] as const)("retries unavailable saved speech (%s) but does not submit an incomplete replacement", async speechCredentialState => {
  vi.mocked(profileCredentialEditorState).mockRejectedValue("credential_store_unavailable");
  const connectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...props.profile, credentialState: "unavailable", speechCredentialState }, connectionCheck });
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined);
  await change("#test-speech-endpoint", "wss://replacement.example/inference");
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-speech-model", "replacement-model");
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-speech-key", "synthetic-replacement-key");
  expect(connectionCheck).toHaveBeenLastCalledWith({ kind: "customSpeech", endpoint: "wss://replacement.example/inference", model: "replacement-model", apiKey: "synthetic-replacement-key" });
  expect(props.onSave).not.toHaveBeenCalled();
});

it("checks a complete explicit speech replacement after the saved metadata read fails", async () => {
  vi.mocked(profileCredentialEditorState).mockRejectedValue("credential_store_unavailable");
  const connectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...props.profile, credentialState: "present", speechCredentialState: "present" }, connectionCheck });
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.editSpeechConfiguration)!.click());
  await change("#test-speech-model", "synthetic-asr-model");
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-speech-endpoint", "wss://speech.example/inference");
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-speech-key", "synthetic-asr-key");
  expect(connectionCheck).toHaveBeenLastCalledWith({ kind: "customSpeech", endpoint: "wss://speech.example/inference", model: "synthetic-asr-model", apiKey: "synthetic-asr-key" });
  expect(props.onSave).not.toHaveBeenCalled();
});

it("checks saved speech model changes with native key reuse and requires a key for a new endpoint", async () => {
  vi.mocked(profileCredentialEditorState).mockResolvedValue({ savedFields: ["apiKey"], endpoint: "wss://saved.example/inference", model: "saved-model" });
  const connectionCheck = vi.fn().mockReturnValue(null);
  await render({ ...props, profile: { ...props.profile, credentialState: "present", speechCredentialState: "present" }, connectionCheck });
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined);
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.editSpeechConfiguration)!.click());
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined);
  await change("#test-speech-model", "edited-model");
  expect(connectionCheck).toHaveBeenLastCalledWith({ kind: "customSpeech", endpoint: "", model: "edited-model", apiKey: "" });
  await change("#test-speech-endpoint", "wss://new.example/inference");
  expect(connectionCheck).toHaveBeenLastCalledWith(null);
  await change("#test-speech-key", "synthetic-new-key");
  expect(connectionCheck).toHaveBeenLastCalledWith({ kind: "customSpeech", endpoint: "wss://new.example/inference", model: "edited-model", apiKey: "synthetic-new-key" });
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.cancel)!.click());
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined);
  expect(props.onSave).not.toHaveBeenCalled();
});
it.each(["en", "zh", "ja"] as const)("separates stage identities and puts protocol requirements in help in %s", async language => {
  setStoredUiLanguage(language); await render();
  expect([...host.querySelectorAll(".service-stage h3")].map(node => node.textContent)).toEqual([I18N.settings.speechRecognition, I18N.settings.textTranslationLabel]);
  expect(host.querySelector('[role="combobox"]')?.textContent).toBe(I18N.settings.customSpeechNoTranslation);
  expect(host.querySelector('.service-stage .settings-help-control__description')?.textContent).toContain(I18N.settings.customSpeechRequirementsDashScope);
  expect(host.querySelectorAll("small, .service-stage p")).toHaveLength(0);
  expect(host.querySelector('[data-provider="alibabaCloud"]')).toBeNull();
  expect(host.querySelector('input[type="password"]')?.getAttribute("aria-describedby")).toContain("address-key");
});
it("saves speech alone without a translation key or synthetic defaults", async () => {
  await render(); expect(host.querySelector<HTMLInputElement>("#test-speech-endpoint")?.value).toBe("");
  expect(host.querySelector('button[type="submit"]')?.hasAttribute("disabled")).toBe(true);
  await fillSpeech(); await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "customSpeech", endpoint: "wss://speech.example/inference", model: "synthetic-asr-model", apiKey: "synthetic-asr-key" });
});
it("keeps drafts on failure and focuses an unsafe address before any save", async () => {
  await render(); await fillSpeech("wss://speech.example?key=synthetic"); await submit();
  expect(props.onSave).not.toHaveBeenCalled(); expect(document.activeElement?.id).toBe("test-speech-endpoint");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.customSpeechEndpointInvalid);
  expect(host.querySelector<HTMLInputElement>("#test-speech-key")?.value).toBe("synthetic-asr-key");
  await change("#test-speech-endpoint", "ws://localhost:1888"); await change("#test-speech-model", "x".repeat(257)); await submit();
  expect(document.activeElement?.id).toBe("test-speech-model"); expect(props.onSave).not.toHaveBeenCalled();
});
it("allows model-only updates but requires a fresh key with a changed speech address", async () => {
  await render({ ...props, profile: { ...props.profile, credentialState: "present", speechCredentialState: "present" } });
  expect(host.querySelector("input")).toBeNull();
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.editSpeechConfiguration)!.click());
  await change("#test-speech-model", "new-model"); await submit();
  expect(props.onSave).toHaveBeenCalledWith({ kind: "customSpeech", endpoint: "", model: "new-model", apiKey: "" });
  await change("#test-speech-endpoint", "wss://new.example/inference");
  expect(host.querySelector<HTMLInputElement>("#test-speech-key")?.required).toBe(true);
  expect(host.querySelector('button[type="submit"]')?.hasAttribute("disabled")).toBe(true);
});
it("configures text translation using its own key while speech drafts remain untouched", async () => {
  await render({ ...props, profile: { ...props.profile, speechCredentialState: "present", textCredentialState: "missing", textTranslation: "openAICompatible" } });
  await change("#test-text-endpoint", "https://translation.example/v1"); await change("#test-text-model", "synthetic-text-model"); await change("#test-text-token", "synthetic-text-key");
  await submit(".service-stage--translation");
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "https://translation.example/v1", model: "synthetic-text-model", token: "synthetic-text-key" });
  expect(host.querySelector("#test-speech-key")).toBeNull();
  for (const input of host.querySelectorAll<HTMLInputElement>("input")) for (const id of (input.getAttribute("aria-describedby") ?? "").split(" ").filter(Boolean)) expect(document.getElementById(id), id).not.toBeNull();
});
it("retains a saved speech stage even when its translation credentials are missing", async () => {
  await render({ ...props, profile: { ...props.profile, speechCredentialState: "present", textCredentialState: "missing", textTranslation: "deepL" } });
  expect(host.querySelector("#test-speech-key")).toBeNull();
  expect(host.querySelector<HTMLInputElement>("#test-text-token")?.value).toBe("");
  await change("#test-text-token", "synthetic-text-key"); await submit("form.service-stages");
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "deepL", endpoint: "", model: "", token: "synthetic-text-key" });
});
it("can save translation before recognition without submitting speech drafts", async () => {
  await render({ ...props, profile: { ...props.profile, textTranslation: "deepL", textCredentialState: "missing" } });
  await change("#test-speech-key", "synthetic-unsaved-speech-key");
  await change("#test-text-token", "synthetic-text-key"); await submit("form.service-stages");
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "alibabaTranslation", apiKey: "", textTranslation: "deepL", endpoint: "", model: "", token: "synthetic-text-key" });
  expect(host.querySelector<HTMLInputElement>("#test-speech-key")?.value).toBe("synthetic-unsaved-speech-key");
});
it("shows OpenAI standalone requirements and locks all forms in an active session", async () => {
  await render({ ...props, disabled: true, profile: { ...props.profile, provider: "customOpenAIASR" } });
  expect(host.querySelector('.service-stage .settings-help-control__description')?.textContent).toContain(I18N.settings.customSpeechRequirementsOpenAI);
  expect([...host.querySelectorAll<HTMLInputElement>("input")].every(input => input.disabled)).toBe(true);
  expect(host.querySelector('[role="combobox"]')?.hasAttribute("disabled")).toBe(true);
});
it("clears replacement speech and text drafts after confirmed deletion", async () => {
  await render(); await fillSpeech(); await render({ ...props, confirmingDelete: true });
  await act(async () => [...document.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.confirmDelete)!.click());
  expect(props.onConfirmDelete).toHaveBeenCalledOnce(); expect(host.querySelector<HTMLInputElement>("#test-speech-key")?.value).toBe("");
});


it("shows saved speech address and model in editable fields and reveals its key in that same input", async () => {
  vi.mocked(profileCredentialEditorState).mockResolvedValue({ savedFields: ["apiKey"], endpoint: "wss://speech.example/inference", model: "saved-asr-model" });
  await render({ ...props, profile: { ...props.profile, credentialState: "present", speechCredentialState: "present" } });
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.editSpeechConfiguration)!.click());
  expect(host.querySelector<HTMLInputElement>("#test-speech-endpoint")!.value).toBe("wss://speech.example/inference");
  expect(host.querySelector<HTMLInputElement>("#test-speech-model")!.value).toBe("saved-asr-model");
  expect(profileRevealCredential).not.toHaveBeenCalled();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-saved-key");
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.revealSavedCredential)!.click());
  expect(host.querySelector<HTMLInputElement>("#test-speech-key")!.value).toBe("synthetic-saved-key");
  expect(host.querySelector<HTMLInputElement>("#test-speech-key")!.readOnly).toBe(false);
  expect(host.querySelectorAll('.service-stage:not(.service-stage--translation) input')).toHaveLength(3);
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
  await change("#test-speech-model", "edited-asr-model");
  await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "customSpeech", endpoint: "", model: "edited-asr-model", apiKey: "" });
});

it.each(["customDashScopeASR", "customOpenAIASR"] as const)("expands long saved %s fields without loading secrets and preserves a model edit", async provider => {
  const endpoint = `wss://synthetic.example/${"speech-path/".repeat(20)}inference`;
  const model = `synthetic-${"speech-model-".repeat(12)}revision`;
  vi.mocked(profileCredentialEditorState).mockResolvedValue({ savedFields: ["apiKey"], endpoint, model });
  await render({ ...props, profile: { ...props.profile, provider, credentialState: "present", speechCredentialState: "present" } });
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.editSpeechConfiguration)!.click());
  const endpointGroup = host.querySelector("#test-speech-endpoint")!.closest(".config-input-group")!;
  await act(() => endpointGroup.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
  expect(endpointGroup.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe(endpoint);
  expect(endpointGroup.querySelector<HTMLTextAreaElement>("textarea")!.readOnly).toBe(false);
  await act(() => endpointGroup.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
  expect(host.querySelector<HTMLInputElement>("#test-speech-endpoint")!.value).toBe(endpoint);
  expect(props.onSave).not.toHaveBeenCalled();

  const modelGroup = host.querySelector("#test-speech-model")!.closest(".config-input-group")!;
  await act(() => modelGroup.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
  expect(modelGroup.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe(model);
  await change("#test-speech-model", `${model}-edited`);
  await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "customSpeech", endpoint: "", model: `${model}-edited`, apiKey: "" });
  expect(modelGroup.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe(`${model}-edited`);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(host.querySelector("#test-speech-key")!.closest(".config-input-group")!.querySelector(".config-input__expand")).toBeNull();
});

it("preserves custom speech edits against a delayed metadata read and clears the revealed key on cancel", async () => {
  let finish!: (value: Awaited<ReturnType<typeof profileCredentialEditorState>>) => void;
  vi.mocked(profileCredentialEditorState).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await render({ ...props, profile: { ...props.profile, credentialState: "present", speechCredentialState: "present" } });
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.editSpeechConfiguration)!.click());
  await change("#test-speech-model", "unsaved-asr-model");
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
  await submit();
  expect(props.onSave).not.toHaveBeenCalled();
  await act(async () => finish({ savedFields: ["apiKey"], endpoint: "wss://speech.example/inference", model: "saved-asr-model" }));
  expect(host.querySelector<HTMLInputElement>("#test-speech-model")!.value).toBe("unsaved-asr-model");
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-saved-key");
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.revealSavedCredential)!.click());
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.cancel)!.click());
  expect(host.querySelector("#test-speech-key")).toBeNull();
  expect(props.onSave).not.toHaveBeenCalled();
});


it("treats unchanged speech address and model as retained values when replacing its key", async () => {
  vi.mocked(profileCredentialEditorState).mockResolvedValue({ savedFields: ["apiKey"], endpoint: "wss://speech.example/inference", model: "saved-asr-model" });
  await render({ ...props, profile: { ...props.profile, credentialState: "present", speechCredentialState: "present" } });
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === I18N.settings.editSpeechConfiguration)!.click());
  await change("#test-speech-endpoint", "wss://speech.example/inference");
  await change("#test-speech-model", "saved-asr-model");
  expect(host.querySelector('button[type="submit"]')!.hasAttribute("disabled")).toBe(true);
  await change("#test-speech-key", "synthetic-new-key");
  await submit();
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ kind: "customSpeech", endpoint: "", model: "", apiKey: "synthetic-new-key" });
  await change("#test-speech-key", "");
  await change("#test-speech-endpoint", "wss://new.example/inference");
  expect(host.querySelector<HTMLInputElement>("#test-speech-key")!.placeholder).toBe(I18N.settings.apiKeyPlaceholder);
  expect(host.querySelector(".service-stage:not(.service-stage--translation) .saved-credential-input__toggle")).toBeNull();
});
