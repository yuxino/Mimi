// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { profileRevealCredential } from "../../lib/ipc";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import { SavedCredentialInput } from "./SavedCredentialInput";
import { SettingsToastRegion } from "./SettingsToast";

const native = vi.hoisted(() => ({ enabled: false, listen: vi.fn() }));
vi.mock("../../lib/ipc", () => ({ get isTauri() { return native.enabled; }, profileRevealCredential: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ listen: native.listen }) }));
let root: Root;
let host: HTMLDivElement;
const changed = vi.fn();
const readClipboard = vi.fn<() => Promise<string>>();
const defaults = { profileId: "synthetic", field: "apiKey" as const, label: "API Key", hasSavedValue: true };
type Props = Partial<Parameters<typeof SavedCredentialInput>[0]>;

function Draft(props: Props) {
  const [value, setValue] = useState("");
  return <SavedCredentialInput {...defaults} value={value} onValueChange={next => { changed(next); setValue(next); }} {...props} />;
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  vi.mocked(profileRevealCredential).mockReset();
  changed.mockReset(); readClipboard.mockReset();
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { readText: readClipboard } });
  native.enabled = false; native.listen.mockReset().mockResolvedValue(vi.fn());
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(() => root.unmount()); host.remove();
  Reflect.deleteProperty(navigator, "clipboard");
  setStoredUiLanguage("en"); vi.restoreAllMocks(); vi.unstubAllGlobals();
});
async function render(props: Props = {}) { await act(() => root.render(<Draft {...props} />)); }
const input = () => host.querySelector<HTMLInputElement>("input")!;
const eye = () => host.querySelector<HTMLButtonElement>(".saved-credential-input__toggle")!;
async function clickEye() { await act(async () => eye().click()); }
async function type(value: string) {
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it.each(["zh", "en", "ja"] as const)("reveals in the same editable input without creating a replacement draft in %s", async language => {
  setStoredUiLanguage(language);
  const storage = vi.spyOn(Storage.prototype, "setItem");
  await render({ id: "synthetic-input" });
  const original = input();
  expect(original.type).toBe("password");
  expect(original.value).toBe("");
  expect(original.placeholder).toBe("••••••••");
  expect(original.getAttribute("aria-description")).toBe(I18N.settings.savedCredential);
  expect(eye().getAttribute("aria-label")).toBe(`${I18N.settings.revealSavedCredential}: API Key`);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-stored-key");
  await clickEye();
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: "synthetic", field: "apiKey" });
  expect(host.querySelectorAll("input")).toHaveLength(1);
  expect(input()).toBe(original);
  expect(input().type).toBe("text");
  expect(input().value).toBe("synthetic-stored-key");
  expect(input().readOnly).toBe(false);
  expect(changed).not.toHaveBeenCalled();
  expect(storage).not.toHaveBeenCalled();
  await clickEye();
  expect(input().type).toBe("password");
  expect(input().value).toBe("");
  expect(changed).not.toHaveBeenCalled();
});

it("has no saved-value action for a legitimately empty optional key, but can show a new draft", async () => {
  await render({ hasSavedValue: false, placeholder: "Optional synthetic key" });
  expect(eye()).toBeNull();
  expect(input().placeholder).toBe("Optional synthetic key");
  expect(profileRevealCredential).not.toHaveBeenCalled();
  await type("synthetic-new-key");
  expect(input().type).toBe("password");
  expect(eye().getAttribute("aria-label")).toBe(`${I18N.settings.showCredential}: API Key`);
  await clickEye();
  expect(input().value).toBe("synthetic-new-key");
  expect(input().type).toBe("text");
  await clickEye();
  expect(input().value).toBe("synthetic-new-key");
  expect(input().type).toBe("password");
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("turns an edited saved value into a draft that hide and blur preserve", async () => {
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-stored-key");
  await render(); await clickEye();
  await type("synthetic-edited-key");
  expect(changed).toHaveBeenCalledExactlyOnceWith("synthetic-edited-key");
  expect(input().value).toBe("synthetic-edited-key");
  expect(input().type).toBe("text");
  await clickEye();
  expect(input().type).toBe("password");
  await clickEye();
  expect(input().value).toBe("synthetic-edited-key");
  expect(profileRevealCredential).toHaveBeenCalledOnce();
  await act(() => window.dispatchEvent(new Event("blur")));
  expect(input().type).toBe("password");
  expect(input().value).toBe("synthetic-edited-key");
});

it("never reads over an unsaved draft and rejects a pending saved read when typing starts", async () => {
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  await render(); await clickEye();
  expect(eye().getAttribute("aria-busy")).toBe("true");
  await type("synthetic-unsaved-key");
  await act(async () => complete("synthetic-late-saved-key"));
  expect(input().value).toBe("synthetic-unsaved-key");
  expect(input().type).toBe("password");
  await clickEye();
  expect(input().type).toBe("text");
  expect(input().value).toBe("synthetic-unsaved-key");
  expect(profileRevealCredential).toHaveBeenCalledOnce();
});

it("pastes into the same draft and rejects a pending saved read", async () => {
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  readClipboard.mockResolvedValue("synthetic-pasted-key");
  await render(); await clickEye();
  await act(async () => host.querySelector<HTMLButtonElement>(".config-input__paste")!.click());
  await act(async () => complete("synthetic-late-saved-key"));
  expect(input().value).toBe("synthetic-pasted-key");
  expect(changed).toHaveBeenCalledExactlyOnceWith("synthetic-pasted-key");
});

it("cancels loading on a second eye click and supports a fresh read", async () => {
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  await render(); await clickEye(); await clickEye();
  await act(async () => complete("synthetic-stale-key"));
  expect(input().value).toBe("");
  expect(input().type).toBe("password");
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-fresh-key");
  await clickEye();
  expect(input().value).toBe("synthetic-fresh-key");
});

it("clears on focus leaving the input group while keeping eye and paste interactions inside it", async () => {
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-stored-key");
  await render(); await clickEye();
  await act(() => input().focus());
  await act(() => eye().focus());
  expect(input().value).toBe("synthetic-stored-key");
  const outside = document.createElement("button"); document.body.append(outside);
  await act(() => outside.focus());
  expect(input().value).toBe("");
  expect(input().type).toBe("password");
  outside.remove();
});

it.each(["blur", "hidden", "escape"] as const)("clears shown values and rejects in-flight reads after %s", async boundary => {
  let hidden = false;
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  const leave = async () => {
    await act(() => {
      if (boundary === "blur") window.dispatchEvent(new Event("blur"));
      else if (boundary === "hidden") { hidden = true; document.dispatchEvent(new Event("visibilitychange")); }
      else input().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
  };
  await render();
  vi.mocked(profileRevealCredential).mockResolvedValueOnce("synthetic-visible-key"); await clickEye();
  await leave();
  expect(input().value).toBe(""); expect(input().type).toBe("password");
  hidden = false;
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  await clickEye(); await leave();
  await act(async () => complete("synthetic-late-key"));
  expect(input().value).toBe(""); expect(changed).not.toHaveBeenCalled();
});

it.each([{ disabled: true }, { active: false }, { hasSavedValue: false }])("clears and invalidates reads when availability changes: %j", async unavailable => {
  await render();
  vi.mocked(profileRevealCredential).mockResolvedValueOnce("synthetic-visible-key"); await clickEye();
  await render(unavailable);
  expect(input().value).toBe(""); expect(input().type).toBe("password");
  await render();
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  await clickEye(); await render(unavailable);
  await act(async () => complete("synthetic-late-key"));
  await render();
  expect(input().value).toBe("");
});

it.each([
  { profileId: "other" },
  { field: "token" as const, textTranslation: "deepLX" as const },
  { value: "synthetic-owner-draft" },
])("rejects saved reads after profile, route or owner draft changes: %j", async next => {
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  await render(); await clickEye(); await render(next);
  await act(async () => complete("synthetic-old-profile-key"));
  expect(input().value).toBe(next.value ?? "");
  expect(input().type).toBe("password");
  expect(changed).not.toHaveBeenCalled();
});

it("uses the selected translation route and sanitizes failed reads without extra inputs", async () => {
  await act(() => root.render(<><Draft field="token" textTranslation="openAICompatible" /><SettingsToastRegion /></>));
  vi.mocked(profileRevealCredential).mockRejectedValue("synthetic-private-rejection");
  await clickEye();
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: "synthetic", field: "token", textTranslation: "openAICompatible" });
  expect(host.querySelector('.settings-toast[role="alert"]')?.textContent).toBe(profileErrorMessage("synthetic-private-rejection"));
  expect(host.textContent).not.toContain("synthetic-private-rejection");
  expect(host.querySelectorAll("input")).toHaveLength(1);
  expect(input().value).toBe("");
});

it("handles stale saved metadata with a localized missing-value toast and no draft mutation", async () => {
  await act(() => root.render(<><Draft /><SettingsToastRegion /></>));
  vi.mocked(profileRevealCredential).mockResolvedValue(null);
  await clickEye();
  expect(host.querySelector('.settings-toast[role="alert"]')?.textContent).toBe(I18N.settings.savedCredentialMissing);
  expect(input().value).toBe("");
  expect(input().type).toBe("password");
  expect(changed).not.toHaveBeenCalled();
});

it.each(["tauri://blur", "tauri://close-requested"])("clears on native %s and ignores late responses without unmounting", async event => {
  native.enabled = true;
  const handlers = new Map<string, () => void>();
  const unlisten = vi.fn();
  native.listen.mockImplementation(async (name, handler) => { handlers.set(name, handler); return unlisten; });
  await render();
  vi.mocked(profileRevealCredential).mockResolvedValueOnce("synthetic-native-key"); await clickEye();
  await act(() => handlers.get(event)!());
  expect(input().value).toBe("");
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  await clickEye(); await act(() => handlers.get(event)!());
  await act(async () => complete("synthetic-late-native-key"));
  expect(input().value).toBe("");
  await act(() => root.render(null));
  expect(unlisten).toHaveBeenCalledTimes(2);
});

it("discards read results and cleans native subscriptions that arrive after unmount", async () => {
  native.enabled = true;
  const subscriptions: ((unlisten: () => void) => void)[] = [];
  native.listen.mockImplementation(() => new Promise(resolve => { subscriptions.push(resolve); }));
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  await render(); await clickEye();
  await act(() => root.render(null));
  const unlisten = vi.fn();
  await act(async () => { subscriptions.forEach(finish => finish(unlisten)); complete("synthetic-unmounted-key"); });
  expect(unlisten).toHaveBeenCalledTimes(2);
  expect(changed).not.toHaveBeenCalled();
  await render();
  expect(input().value).toBe("");
});
