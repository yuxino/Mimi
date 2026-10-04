// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { OverlayControlPanel } from "./OverlayControlPanel";
import { overlayControlPanelModel } from "./overlayControlModel";
import { sourceLanguagesForSettings } from "../../lib/providerCapabilities";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, type SettingsSnapshot } from "../../lib/types";

let host: HTMLDivElement;
let root: Root;
let props: Parameters<typeof OverlayControlPanel>[0];
const scrollIntoView = Object.getOwnPropertyDescriptor(Element.prototype, "scrollIntoView");

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const settings: SettingsSnapshot = { ...useStore.getState().settings, sourceLanguage: "auto", targetLanguage: "zh", languageCapabilities: undefined,
    profiles: [{ id: "ali", name: "Alibaba Cloud", provider: "alibabaCloud", credentialState: "present" }], activeProfileId: "ali" };
  props = {
    settings, model: overlayControlPanelModel(settings), phase: "listening",
    status: { source: "Automatic", separator: "→", target: "Chinese" },
    isPaused: false, canPauseSession: true, isWaitingForFinalTranslation: false, isChangingSession: false,
    onDismiss: vi.fn(), onSwitchSourceLanguage: vi.fn().mockResolvedValue(undefined),
    onTogglePaused: vi.fn().mockResolvedValue(undefined),
    onSetSkipTranslation: vi.fn().mockResolvedValue(undefined),
    onSetIntermediateSubtitles: vi.fn().mockResolvedValue(undefined),
    onSetSubtitleTimestamps: vi.fn().mockResolvedValue(undefined),
    onSetSubtitleDisplayMode: vi.fn().mockResolvedValue(undefined), onSetImmersiveMode: vi.fn().mockResolvedValue(undefined),
    onSetOverlayLocked: vi.fn().mockResolvedValue(undefined), onShowSettings: vi.fn().mockResolvedValue(undefined),
  };
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  setStoredUiLanguage("system"); vi.unstubAllGlobals();
  if (scrollIntoView) Object.defineProperty(Element.prototype, "scrollIntoView", scrollIntoView);
  else Reflect.deleteProperty(Element.prototype, "scrollIntoView");
});
async function mount() { await act(async () => root.render(<OverlayControlPanel {...props} />)); }
function picker(label: string) { return host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${label}"]`)!; }
async function key(node: HTMLElement, value: string) {
  await act(async () => node.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true })));
}
function configure(draft: Partial<SettingsSnapshot>) {
  props.settings = { ...props.settings, ...draft };
  props.model = overlayControlPanelModel(props.settings);
}
async function searchSource(query: string) {
  await act(async () => picker(I18N.overlay.sourceLanguage).click());
  const search = document.querySelector<HTMLInputElement>("input.mimi-select__search")!;
  expect(search.getAttribute("aria-label")).toBe(I18N.settings.searchLanguages);
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, query);
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return search;
}

it.each(["zh", "en", "ja"] as const)("keeps %s language, display and application choices compact without a mode grid", async (language) => {
  setStoredUiLanguage(language);
  await mount();
  expect(host.querySelectorAll('[role="combobox"]')).toHaveLength(3);
  expect(host.querySelectorAll('.application-audio-picker')).toHaveLength(1);
  expect(host.querySelector('fieldset, .overlay-control-options, .overlay-control-group')).toBeNull();
  expect(host.querySelectorAll('[role="switch"]')).toHaveLength(5);
  expect(host.querySelector('.overlay-control-setting small')).toBeNull();
  expect(picker(I18N.overlay.sourceLanguage)).toBe(document.activeElement);
  expect(props.onSwitchSourceLanguage).not.toHaveBeenCalled();
  expect(props.onSetSubtitleDisplayMode).not.toHaveBeenCalled();
});

it("supports keyboard source selection and dismisses only after the command succeeds", async () => {
  await mount();
  const source = picker(I18N.overlay.sourceLanguage);
  await key(source, "ArrowDown"); await key(source, "End"); await key(source, "Enter");
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith(props.model.sourceOptions.at(-1));
  expect(props.onDismiss).toHaveBeenCalledOnce();
});

it("keeps system audio application selection and More settings without hidden input switches", async () => {
  await mount();
  const audioControls = host.querySelector(".overlay-control-capture")!;
  expect(audioControls.querySelectorAll('[role="switch"]')).toHaveLength(0);
  expect(audioControls.querySelector('[data-audio-source="microphone"]')).toBeNull();
  expect(audioControls.querySelector('.application-audio-picker')).not.toBeNull();
  expect(audioControls.querySelector(`button[aria-label="${I18N.settings.audioInputTitle}"]`)).toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>(".overlay-control-settings-link")!.click());
  expect(props.onShowSettings).toHaveBeenCalledExactlyOnceWith();
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("changes subtitle display without closing the panel and lets Escape close only its picker", async () => {
  await mount();
  const display = picker(I18N.settings.subtitleDisplay);
  await key(display, "ArrowDown"); await key(display, "Escape");
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(props.onDismiss).not.toHaveBeenCalled();
  await key(display, "ArrowDown"); await key(display, "End"); await key(display, "Enter");
  expect(props.onSetSubtitleDisplayMode).toHaveBeenCalledOnce();
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("blocks conflicting operations and retains an inline error after a failed toggle", async () => {
  let reject!: (reason: Error) => void;
  props.onSetImmersiveMode = vi.fn(() => new Promise<void>((_resolve, failure) => { reject = failure; }));
  await mount();
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.overlay.immersiveMode}"]`)!.click());
  expect([...host.querySelectorAll<HTMLButtonElement>('[role="combobox"], [role="switch"]')].every((button) => button.disabled)).toBe(true);
  await act(async () => reject(new Error("synthetic-toggle-failure")));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.overlay.controlActionFailed);
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("keeps recognition locked during transitions while independent display remains available", async () => {
  props.isChangingSession = true;
  await mount();
  expect(picker(I18N.overlay.sourceLanguage).disabled).toBe(true);
  expect(picker(I18N.settings.subtitleDisplay).disabled).toBe(false);
  expect(host.querySelector<HTMLButtonElement>(".overlay-control-session-action")!.disabled).toBe(true);
});

it("keeps pause unavailable after the session has stopped", async () => {
  props.canPauseSession = false;
  await mount();
  const pause = host.querySelector<HTMLButtonElement>(".overlay-control-session-action")!;
  expect(pause.disabled).toBe(true);
  await act(async () => pause.click());
  expect(props.onTogglePaused).not.toHaveBeenCalled();
});

it("keeps pause and resume in the immersive panel, blocks duplicate actions, and reports a failed retry", async () => {
  configure({ subtitleBlendsWithBackground: true, audioInput: "microphone" });
  let resolve!: () => void;
  props.onTogglePaused = vi.fn(() => new Promise<void>(done => { resolve = done; }));
  await mount();
  const action = () => host.querySelector<HTMLButtonElement>(".overlay-control-session-action")!;
  expect(action().textContent).toBe(I18N.overlay.pause);
  await act(async () => action().click());
  expect(action().disabled).toBe(true);
  expect(host.querySelector('[role="dialog"]')?.getAttribute("aria-busy")).toBe("true");
  await act(async () => action().click());
  expect(props.onTogglePaused).toHaveBeenCalledTimes(1);
  await act(async () => resolve());
  props.isPaused = true;
  await mount();
  expect(action().textContent).toBe(I18N.overlay.resume);
  expect(props.onDismiss).not.toHaveBeenCalled();
  props.onTogglePaused = vi.fn().mockRejectedValueOnce(new Error("synthetic-private-resume-error")).mockResolvedValue(undefined);
  await mount();
  await act(async () => action().click());
  expect(host.querySelector('.overlay-control-alert[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  expect(host.textContent).not.toContain("synthetic-private-resume-error");
  expect(action().textContent).toBe(I18N.overlay.resume);
  await act(async () => action().click());
  expect(host.querySelector('.overlay-control-alert')).toBeNull();
  expect(props.onTogglePaused).toHaveBeenCalledTimes(2);
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it.each(["zh", "en", "ja"] as const)("offers the same full source list as settings and searches French in %s", async locale => {
  setStoredUiLanguage(locale);
  await mount();
  await act(async () => picker(I18N.overlay.sourceLanguage).click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options).toHaveLength(25);
  expect(options.map(option => option.textContent)).toEqual(sourceLanguagesForSettings(props.settings)
    .map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]));
  expect(document.querySelector("input.mimi-select__search")).not.toBeNull();
  await key(document.querySelector<HTMLInputElement>("input.mimi-select__search")!, "Escape");
  await searchSource("fr");
  const filtered = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(filtered).toHaveLength(1);
  expect(filtered[0].textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  await act(async () => filtered[0].click());
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith("fr");
  expect(props.onDismiss).toHaveBeenCalledOnce();
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});

it("offers all Original-mode recognition hints including searchable Norwegian", async () => {
  configure({ targetLanguage: "original" });
  await mount();
  await act(async () => picker(I18N.overlay.sourceLanguage).click());
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(31);
  const search = document.querySelector<HTMLInputElement>("input.mimi-select__search")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "Norwegian");
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const option = document.querySelector<HTMLElement>('[role="option"]')!;
  expect(option.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.no);
  await act(async () => option.click());
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith("no");
});

it.each(["deepL", "deepLX", "openAICompatible"] as const)("keeps the %s small route list non-searchable and matches settings language labels", async route => {
  configure({ profiles: [{ ...props.settings.profiles[0], textTranslation: route }] });
  await mount();
  await act(async () => picker(I18N.overlay.sourceLanguage).click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options).toHaveLength(5);
  expect(options.map(option => option.textContent)).toEqual(sourceLanguagesForSettings(props.settings)
    .map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]));
  expect(document.querySelector("input.mimi-select__search")).toBeNull();
  const chinese = options.find(option => option.textContent === SOURCE_LANGUAGE_DISPLAY_NAMES.zh)!;
  await act(async () => chinese.click());
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith("zh");
});

it("uses native source choices without search for six options and ignores a stale route stamp", async () => {
  const native = { profileId: "ali", provider: "alibabaCloud", textTranslation: "followService", targetLanguage: "zh",
    sourceLanguages: ["auto", "ja", "en", "ko", "zh", "fr"], targetLanguages: ["original", "zh", "fr"] } as const;
  configure({ sourceLanguage: "fr", languageCapabilities: native });
  await mount();
  const source = picker(I18N.overlay.sourceLanguage);
  expect(source.textContent).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  await act(async () => source.click());
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(6);
  expect(document.querySelector("input.mimi-select__search")).toBeNull();
  expect(document.querySelector('[role="option"][aria-selected="true"]')?.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  await key(source, "Escape");
  configure({ languageCapabilities: { ...native, profileId: "removed" } });
  await mount();
  await act(async () => picker(I18N.overlay.sourceLanguage).click());
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(25);
  expect(document.querySelector("input.mimi-select__search")).not.toBeNull();
});

it("reports an empty language search and Escape returns focus without issuing a command", async () => {
  await mount();
  const search = await searchSource("no-matching-language");
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(0);
  expect(document.querySelector('.mimi-select__empty[role="status"]')?.textContent).toBe(I18N.settings.noMatchingLanguages);
  await key(search, "Escape");
  expect(document.activeElement).toBe(picker(I18N.overlay.sourceLanguage));
  expect(props.onSwitchSourceLanguage).not.toHaveBeenCalled();
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("offers recognition-only without closing the live panel and reflects the saved target", async () => {
  await mount();
  const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.skipTranslation}"]`)!;
  expect(toggle().getAttribute("aria-checked")).toBe("false");
  await act(async () => toggle().click());
  expect(props.onSetSkipTranslation).toHaveBeenCalledExactlyOnceWith(true);
  expect(props.onDismiss).not.toHaveBeenCalled();
  configure({ targetLanguage: "original" });
  await mount();
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  await act(async () => toggle().click());
  expect(props.onSetSkipTranslation).toHaveBeenLastCalledWith(false);
});

it("locks skipping during a reconnect and hides it for integrated providers without Original", async () => {
  props.isChangingSession = true;
  await mount();
  expect(host.querySelector<HTMLButtonElement>(`[aria-label="${I18N.settings.skipTranslation}"]`)!.disabled).toBe(true);
  configure({ profiles: [{ id: "ali", provider: "openAIRealtime", name: "OpenAI", credentialState: "present" }] });
  await mount();
  expect(host.querySelector(`[aria-label="${I18N.settings.skipTranslation}"]`)).toBeNull();
});


it.each([true, false, undefined])("changes early subtitle display from saved %s without closing or restarting the session", async enabled => {
  configure({ showIntermediateSubtitles: enabled });
  props.isChangingSession = true;
  props.isPaused = true;
  await mount();
  const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.showIntermediateSubtitles}"]`)!;
  expect(toggle().getAttribute("aria-checked")).toBe(String(enabled !== false));
  expect(toggle().disabled).toBe(false);
  await act(async () => toggle().click());
  expect(props.onSetIntermediateSubtitles).toHaveBeenCalledExactlyOnceWith(enabled === false);
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(props.onSetSkipTranslation).not.toHaveBeenCalled();
  expect(props.onSetImmersiveMode).not.toHaveBeenCalled();
  configure({ showIntermediateSubtitles: enabled === false });
  await mount();
  expect(toggle().getAttribute("aria-checked")).toBe(String(enabled === false));
});

it("retains the saved subtitle preference after failure, blocks duplicate saves and allows retry", async () => {
  let reject!: (reason: Error) => void;
  props.onSetIntermediateSubtitles = vi.fn()
    .mockImplementationOnce(() => new Promise<void>((_resolve, failure) => { reject = failure; }))
    .mockResolvedValue(undefined);
  configure({ showIntermediateSubtitles: true });
  await mount();
  const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.showIntermediateSubtitles}"]`)!;
  await act(async () => { toggle().click(); toggle().click(); });
  expect(props.onSetIntermediateSubtitles).toHaveBeenCalledTimes(1);
  expect(toggle().disabled).toBe(true);
  await act(async () => reject(new Error("private synthetic error")));
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  expect(host.textContent).not.toContain("private synthetic error");
  await act(async () => toggle().click());
  expect(props.onSetIntermediateSubtitles).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(props.onDismiss).not.toHaveBeenCalled();
});
