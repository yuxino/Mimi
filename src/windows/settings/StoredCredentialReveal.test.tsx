// @vitest-environment jsdom
import { act } from "react";
import { SettingsToastRegion } from "./SettingsToast";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { profileRevealCredential } from "../../lib/ipc";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import { StoredCredentialReveal } from "./StoredCredentialReveal";

const native = vi.hoisted(() => ({ enabled: false, listen: vi.fn() }));
vi.mock("../../lib/ipc", () => ({ get isTauri() { return native.enabled; }, profileRevealCredential: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ listen: native.listen }) }));
let root: Root;
let host: HTMLDivElement;
const defaults = { profileId: "synthetic", field: "apiKey" as const, label: "API Key", disabled: false };
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  vi.mocked(profileRevealCredential).mockReset();
  native.enabled = false; native.listen.mockReset().mockResolvedValue(vi.fn());
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); setStoredUiLanguage("en"); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function render(props: Parameters<typeof StoredCredentialReveal>[0] = defaults) { await act(() => root.render(<StoredCredentialReveal {...props} />)); }
async function click() { await act(async () => host.querySelector<HTMLButtonElement>("button")!.click()); }

it.each(["zh", "en", "ja"] as const)("reads only on explicit request, offers a selectable saved value, and clears it on hide in %s", async (language) => {
  setStoredUiLanguage(language);
  const storage = vi.spyOn(Storage.prototype, "setItem");
  await render();
  expect(host.textContent).toBe(I18N.settings.revealSavedCredential);
  expect(host.querySelector("button > span")?.classList.contains("settings-sr-only")).toBe(false);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-stored-key");
  await click();
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: "synthetic", field: "apiKey" });
  const input = host.querySelector<HTMLInputElement>("input")!;
  expect(input.value).toBe("synthetic-stored-key");
  expect(input.readOnly).toBe(true);
  expect(input.getAttribute("aria-label")).toBe(`API Key: ${I18N.settings.savedCredential}`);
  expect(storage).not.toHaveBeenCalled();
  await click();
  expect(host.querySelector("input")).toBeNull();
  expect(host.textContent).toBe(I18N.settings.revealSavedCredential);
  storage.mockRestore();
});

it("cancels a pending reveal and ignores its late response, then allows a fresh retry", async () => {
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  await render(); await click();
  expect(host.querySelector("button")?.getAttribute("aria-busy")).toBe("true");
  await click();
  await act(() => complete("synthetic-late-secret"));
  expect(host.querySelector("input")).toBeNull();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-new-secret");
  await click();
  expect(host.querySelector<HTMLInputElement>("input")?.value).toBe("synthetic-new-secret");
});

it("clears a previous profile or route and discards a response arriving after navigation", async () => {
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  await render({ ...defaults, field: "token", textTranslation: "deepL" }); await click();
  expect(profileRevealCredential).toHaveBeenCalledWith({ profileId: "synthetic", field: "token", textTranslation: "deepL" });
  await render({ ...defaults, profileId: "other", field: "token", textTranslation: "deepLX" });
  await act(() => complete("synthetic-old-route-key"));
  expect(host.querySelector("input")).toBeNull();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-custom-token"); await click();
  expect(host.querySelector<HTMLInputElement>("input")?.value).toBe("synthetic-custom-token");
  await act(() => root.render(null));
  await render();
  expect(host.querySelector("input")).toBeNull();
});

it("shows safe missing/error feedback as a toast and never renders a raw rejected value", async () => {
  await act(() => root.render(<><StoredCredentialReveal {...defaults} /><SettingsToastRegion /></>));
  vi.mocked(profileRevealCredential).mockResolvedValue(null); await click();
  expect(host.querySelector('.settings-toast[role="alert"]')?.textContent).toBe(I18N.settings.savedCredentialMissing);
  expect(host.querySelector(".credential-unavailable")).toBeNull();
  vi.mocked(profileRevealCredential).mockRejectedValue("synthetic-sensitive-rejection"); await click();
  expect(host.querySelector('.settings-toast[role="alert"]')?.textContent).toBe(profileErrorMessage("synthetic-sensitive-rejection"));
  expect(host.textContent).not.toContain("synthetic-sensitive-rejection");
  expect(host.querySelector("input")).toBeNull();
});

it("allows keyboard Escape to clear a visible secret even after controls become disabled", async () => {
  await render({ ...defaults, disabled: true });
  expect(host.querySelector<HTMLButtonElement>("button")!.disabled).toBe(true);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  await render();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-stored-key"); await click();
  await render({ ...defaults, disabled: true });
  expect(host.querySelector<HTMLButtonElement>("button")!.disabled).toBe(false);
  await act(() => host.querySelector<HTMLInputElement>("input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(host.querySelector("input")).toBeNull();
});

it("clears a shown value on window blur and ignores an in-flight read after losing focus", async () => {
  await render();
  vi.mocked(profileRevealCredential).mockResolvedValueOnce("synthetic-visible-secret"); await click();
  await act(() => window.dispatchEvent(new Event("blur")));
  expect(host.querySelector("input")).toBeNull();
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; })); await click();
  await act(() => window.dispatchEvent(new Event("blur")));
  await act(async () => { complete("synthetic-late-secret"); });
  await act(() => window.dispatchEvent(new Event("focus")));
  expect(host.querySelector("input")).toBeNull();
  expect(host.textContent).toBe(I18N.settings.revealSavedCredential);
  expect(profileRevealCredential).toHaveBeenCalledTimes(2);
});

it("clears a preview when the document becomes hidden and keeps reopening hidden", async () => {
  await render();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-visible-secret"); await click();
  const visibility = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
  await act(() => document.dispatchEvent(new Event("visibilitychange")));
  visibility.mockReturnValue(false);
  await act(() => document.dispatchEvent(new Event("visibilitychange")));
  expect(host.querySelector("input")).toBeNull();
  expect(host.textContent).toBe(I18N.settings.revealSavedCredential);
  expect(profileRevealCredential).toHaveBeenCalledOnce();
});

it("clears on native close without unmounting and discards a late result before reopening", async () => {
  native.enabled = true;
  let close!: () => void;
  const unlisten = vi.fn();
  native.listen.mockImplementation(async (_event, handler) => { close = handler; return unlisten; });
  await render();
  expect(native.listen).toHaveBeenCalledWith("tauri://close-requested", expect.any(Function));
  vi.mocked(profileRevealCredential).mockResolvedValueOnce("synthetic-visible-secret"); await click();
  await act(() => close());
  expect(host.querySelector("input")).toBeNull();
  let complete!: (value: string) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; })); await click();
  await act(() => close());
  await act(async () => { complete("synthetic-late-secret"); });
  await render();
  expect(host.querySelector("input")).toBeNull();
  expect(host.textContent).toBe(I18N.settings.revealSavedCredential);
  await act(() => root.render(null));
  expect(unlisten).toHaveBeenCalledOnce();
});

it("cleans a native close subscription that arrives after the field unmounts", async () => {
  native.enabled = true;
  let finish!: (unlisten: () => void) => void;
  native.listen.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  await render();
  await act(() => root.render(null));
  const unlisten = vi.fn();
  await act(async () => { finish(unlisten); });
  expect(unlisten).toHaveBeenCalledOnce();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});
