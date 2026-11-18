// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import languageCatalogs from "../../../../shared/provider-language-catalogs.json";
import { languageStatus } from "../../overlay/timeline/overlayModel";
import { I18N, setStoredUiLanguage } from "../../../lib/i18n";
import { profileErrorMessage } from "../../../lib/providers/connectionDiagnostics";
import { useStore } from "../../../lib/store";
import { OverlayControlPanel } from "./OverlayControlPanel";
import { overlayControlPanelModel } from "./overlayControlModel";
import { sourceLanguagesForSettings } from "../../../lib/providers/providerCapabilities";
import { profileLanguagePresetLabel } from "../../../lib/profiles/profileLanguagePreset";
import { SUBTITLE_DISPLAY_OPTIONS } from "../../../lib/subtitles/subtitleDisplay";
import { AUDIO3_RECOGNITION_LANGUAGE_CODES, SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type SettingsSnapshot } from "../../../lib/types";

const native = vi.hoisted(() => ({ active: false, move: vi.fn() }));
vi.mock("../../../lib/ipc", async original => ({
  ...await original<typeof import("../../../lib/ipc")>(),
  get isTauri() { return native.active; },
  overlayMoveStart: native.move,
  overlayControlSetPanelHeight: async () => {},
  appDesktopShortcutCommands: async () => null,
}));

let host: HTMLDivElement;
let root: Root;
let props: Parameters<typeof OverlayControlPanel>[0];
const scrollIntoView = Object.getOwnPropertyDescriptor(Element.prototype, "scrollIntoView");

beforeEach(() => {
  native.active = false;
  native.move.mockReset().mockResolvedValue(undefined);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const settings: SettingsSnapshot = { ...useStore.getState().settings, sourceLanguage: "auto", targetLanguage: "zh", languageCapabilities: undefined,
    profiles: [{ id: "ali", name: "Alibaba Cloud", provider: "alibabaCloud", credentialState: "present" }], activeProfileId: "ali" };
  props = {
    settings, model: overlayControlPanelModel(settings), phase: "listening",
    status: { source: "Automatic", separator: "→", target: "Chinese" },
    isPaused: false, isWaitingForFinalTranslation: false, isChangingSession: false,
    onDismiss: vi.fn(), onSwitchSourceLanguage: vi.fn().mockResolvedValue(undefined),
    onSwitchTargetLanguage: vi.fn().mockResolvedValue(undefined),
    onSelectProfile: vi.fn().mockResolvedValue(undefined),
    onSetSubtitleDisplayMode: vi.fn().mockResolvedValue(undefined),
    onShowSettings: vi.fn().mockResolvedValue(undefined),
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
function displayChoices() {
  return [...host.querySelectorAll<HTMLButtonElement>(`[role="group"][aria-label="${I18N.settings.subtitleDisplay}"] button[aria-pressed]`)];
}
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

async function press(target: Element, button = 0, clientX = 0) {
  const event = new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button, clientX });
  Object.defineProperties(event, { pointerType: { value: "mouse" }, isPrimary: { value: true } });
  await act(() => { target.dispatchEvent(event); });
  return event;
}

it("moves from header, section text and blank padding independently of collapse", async () => {
  native.active = true;
  configure({ isOverlayLocked: false, subtitleBlendsWithBackground: false });
  await mount();
  const label = host.querySelector(".overlay-control-header__summary strong")!;
  const blank = host.querySelector(".overlay-control-panel__content")!;
  const collapse = host.querySelector<HTMLButtonElement>(".overlay-control-header__collapse")!;
  expect(collapse.getAttribute("aria-label")?.endsWith(I18N.overlay.closeControls)).toBe(true);
  expect(collapse.tabIndex).toBe(0);
  const focused = document.activeElement;

  expect((await press(label)).defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(focused);
  expect(native.move).toHaveBeenCalledOnce();
  await act(() => { document.body.dispatchEvent(new MouseEvent("mouseup", { bubbles: true })); });
  expect((await press(blank)).defaultPrevented).toBe(true);
  expect(native.move).toHaveBeenCalledTimes(2);
  expect((await press(host.querySelector('.overlay-control-section__title')!)).defaultPrevented).toBe(true);
  expect((await press(host.querySelector('.overlay-control-panel')!)).defaultPrevented).toBe(true);
  expect(native.move).toHaveBeenCalledTimes(4);
  expect(props.onDismiss).not.toHaveBeenCalled();

  expect((await press(label, 2)).defaultPrevented).toBe(false);
  expect((await press(collapse)).defaultPrevented).toBe(false);
  await act(() => collapse.click());
  expect(props.onDismiss).toHaveBeenCalledOnce();
  expect(native.move).toHaveBeenCalledTimes(4);
});

it("keeps controls, the portaled picker, and wheel scrolling free of drag", async () => {
  native.active = true;
  configure({ isOverlayLocked: false, subtitleBlendsWithBackground: false });
  await mount();
  const target = picker(I18N.settings.translateTo);
  const panel = host.querySelector(".overlay-control-panel")!;
  for (const control of [target, displayChoices()[0],
    host.querySelector(".overlay-control-settings-link")!]) {
    await press(control);
  }
  await act(() => target.click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')]
    .find(node => node.textContent === TARGET_LANGUAGE_DISPLAY_NAMES.ja)!;
  expect(option.closest(".mimi-select__menu")?.parentElement).toBe(panel);
  expect(panel.querySelector(".overlay-control-panel__content")?.contains(option)).toBe(false);
  await press(option);
  await act(() => option.click());
  expect(props.onSwitchTargetLanguage).toHaveBeenCalledExactlyOnceWith("ja");
  const wheel = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: 120 });
  await act(() => { panel.dispatchEvent(wheel); });
  expect(wheel.defaultPrevented).toBe(false);
  expect(native.move).not.toHaveBeenCalled();
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("preserves the panel scrollbar even when its content exceeds the height limit", async () => {
  native.active = true;
  configure({ isOverlayLocked: false, subtitleBlendsWithBackground: false });
  await mount();
  const panel = host.querySelector<HTMLElement>('.overlay-control-panel')!;
  Object.defineProperties(panel, { scrollHeight: { value: 700 }, clientHeight: { value: 520 }, offsetWidth: { value: 280 }, clientWidth: { value: 280 } });
  vi.spyOn(panel, 'getBoundingClientRect').mockReturnValue({ left: 0, right: 280 } as DOMRect);
  expect((await press(panel, 0, 277)).defaultPrevented).toBe(false);
  expect(native.move).not.toHaveBeenCalled();
  expect((await press(panel, 0, 140)).defaultPrevented).toBe(true);
  expect(native.move).toHaveBeenCalledOnce();
});

it("leaves the chevron usable while movement is unavailable or a display choice is saving", async () => {
  native.active = true;
  configure({ isOverlayLocked: false, subtitleBlendsWithBackground: false, subtitleDisplayMode: "translation" });
  let finishDisplay!: () => void;
  props.onSetSubtitleDisplayMode = vi.fn(() => new Promise<void>(resolve => { finishDisplay = resolve; }));
  await mount();
  const summary = host.querySelector(".overlay-control-header__summary")!;
  const blank = host.querySelector(".overlay-control-panel__content")!;
  await act(() => displayChoices()[1].click());
  const expectNoDrag = async () => {
    expect((await press(summary)).defaultPrevented).toBe(false);
    expect((await press(blank)).defaultPrevented).toBe(false);
    expect(native.move).not.toHaveBeenCalled();
  };
  await expectNoDrag();
  await act(() => finishDisplay());
  expect(props.onDismiss).not.toHaveBeenCalled();
  configure({ isOverlayLocked: true });
  await mount();
  await expectNoDrag();
  configure({ isOverlayLocked: false, subtitleBlendsWithBackground: true });
  await mount();
  await expectNoDrag();
  await act(() => host.querySelector<HTMLButtonElement>(".overlay-control-header__collapse")!.click());
  expect(props.onDismiss).toHaveBeenCalledOnce();
});

it.each(["zh", "zh-TW", "en", "ja", "ko", "fr", "de", "ru", "th"] as const)("groups %s language, audio and subtitle controls in reading order", async (language) => {
  setStoredUiLanguage(language);
  await mount();
  const [languageSection, audioSection, subtitleSection] = host.querySelectorAll("section.overlay-control-section");
  expect(languageSection.getAttribute("aria-label")).toBe(I18N.settings.voiceTranslation);
  expect(languageSection.contains(picker(I18N.overlay.sourceLanguage))).toBe(true);
  const languagePair = languageSection.querySelector(".overlay-control-language-pair")!;
  expect([...languagePair.querySelectorAll('[role="combobox"]')]).toEqual([
    picker(I18N.overlay.sourceLanguage), picker(I18N.settings.translateTo),
  ]);
  expect(audioSection.querySelector("h2")?.textContent).toBe(I18N.settings.audioInputTitle);
  expect(subtitleSection.querySelector("h2")?.textContent).toBe(I18N.settings.subtitleTitle);
  expect(host.querySelectorAll('[role="combobox"]')).toHaveLength(4);
  expect(displayChoices().map(choice => choice.textContent)).toEqual(SUBTITLE_DISPLAY_OPTIONS.map(option => option.shortLabel));
  expect(displayChoices().map(choice => document.getElementById(choice.getAttribute("aria-describedby")!)?.textContent)).toEqual(SUBTITLE_DISPLAY_OPTIONS.map(option => option.label));
  expect(host.querySelectorAll('.application-audio-picker')).toHaveLength(1);
  expect(host.querySelector('fieldset, .overlay-control-options, .overlay-control-group')).toBeNull();
  expect(picker(I18N.overlay.sourceLanguage)).toBe(document.activeElement);
  expect(props.onSwitchSourceLanguage).not.toHaveBeenCalled();
  expect(props.onSetSubtitleDisplayMode).not.toHaveBeenCalled();
});

it("supports keyboard source selection and keeps both language controls available", async () => {
  await mount();
  const source = picker(I18N.overlay.sourceLanguage);
  await key(source, "ArrowDown"); await key(source, "End"); await key(source, "Enter");
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith(props.model.sourceOptions.at(-1));
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("keeps system audio application selection and More settings without hidden input switches", async () => {
  await mount();
  const audioControls = host.querySelector(".overlay-control-capture")!;
  expect(audioControls.querySelectorAll('[role="switch"]')).toHaveLength(0);
  expect(audioControls.querySelector('[data-audio-source="microphone"]')).toBeNull();
  expect(audioControls.querySelector('.application-audio-picker')).not.toBeNull();
  expect(audioControls.querySelector(`button[aria-label="${I18N.settings.audioInputTitle}"]`)).toBeNull();
  const footer = host.querySelector(".overlay-control-footer")!;
  const [settings] = footer.querySelectorAll("button");
  expect(footer.querySelectorAll("button")).toHaveLength(1);
  expect(settings.textContent).toBe(I18N.overlay.moreSettings);
  await act(async () => settings.click());
  expect(props.onShowSettings).toHaveBeenCalledExactlyOnceWith();
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("changes subtitle display directly without closing the panel", async () => {
  configure({ subtitleDisplayMode: "translation" });
  await mount();
  const [translation, bilingual, original] = displayChoices();
  expect([translation, bilingual, original].map(choice => choice.getAttribute("aria-pressed"))).toEqual(["true", "false", "false"]);
  await act(async () => translation.click());
  expect(props.onSetSubtitleDisplayMode).not.toHaveBeenCalled();
  await act(async () => bilingual.click());
  expect(props.onSetSubtitleDisplayMode).toHaveBeenCalledExactlyOnceWith("bilingual");
  configure({ subtitleDisplayMode: "bilingual" });
  await mount();
  expect(bilingual.getAttribute("aria-pressed")).toBe("true");
  expect(translation.getAttribute("aria-pressed")).toBe("false");
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("blocks conflicting operations and keeps the previous subtitle display when a save fails", async () => {
  configure({ subtitleDisplayMode: "translation" });
  let reject!: (reason: Error) => void;
  props.onSetSubtitleDisplayMode = vi.fn(() => new Promise<void>((_resolve, failure) => { reject = failure; }));
  await mount();
  const [translation, bilingual] = displayChoices();
  const settings = host.querySelector<HTMLButtonElement>(".overlay-control-settings-link")!;
  await act(() => bilingual.click());
  for (const picker of host.querySelectorAll<HTMLButtonElement>('[role="combobox"]')) {
    expect(picker.matches(':disabled, [aria-disabled="true"]')).toBe(true);
    await act(() => picker.click());
  }
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect([...displayChoices(), settings].every(button => button.disabled)).toBe(true);
  await act(() => reject(new Error("synthetic-display-failure")));
  expect(translation.getAttribute("aria-pressed")).toBe("true");
  expect(bilingual.disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.overlay.controlActionFailed);
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("does not dismiss a reopened panel when an old recovery navigation completes", async () => {
  props.sessionErrorMessage = I18N.settings.sessionError;
  let finish!: () => void;
  const operation = vi.fn(() => new Promise<void>(resolve => { finish = resolve; }));
  props.onShowSettings = operation;
  await mount();
  await act(() => host.querySelector<HTMLButtonElement>(".session-error-feedback__action")!.click());
  expect(operation).toHaveBeenCalledExactlyOnceWith("service");
  await act(async () => root.render(null));
  await mount();
  await act(async () => finish());
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(host.querySelector('[role="dialog"]')).not.toBeNull();
});

it("keeps recognition locked during transitions while independent display remains available", async () => {
  props.isChangingSession = true;
  await mount();
  expect(picker(I18N.overlay.sourceLanguage).getAttribute("aria-disabled")).toBe("true");
  expect(displayChoices().every(choice => !choice.disabled)).toBe(true);
  expect(host.querySelector(".overlay-control-session-action")).toBeNull();
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
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});

it.each(["zh", "en", "ja"] as const)("keeps Apple source choices and explicit-language help consistent in %s", async locale => {
  setStoredUiLanguage(locale);
  configure({ sourceLanguage: "en", targetLanguage: "original", activeProfileId: "apple",
    profiles: [{ id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "missing" }],
    languageCapabilities: { profileId: "apple", provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original",
      sourceLanguages: ["en", "fr"], targetLanguages: ["original"] } });
  await mount();
  const source = picker(I18N.overlay.sourceLanguage);
  const help = source.closest('.overlay-control-picker')!.querySelector('.settings-help-control__description')!;
  expect(help.textContent).toBe(I18N.settings.appleSpeechLanguageHelp);
  expect(help.textContent).not.toBe(I18N.settings.recognitionHintHelp);
  await act(async () => source.click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options.map(option => option.textContent)).toEqual([SOURCE_LANGUAGE_DISPLAY_NAMES.en, SOURCE_LANGUAGE_DISPLAY_NAMES.fr]);
  await act(async () => options[1].click());
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith("fr");
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

it.each(["deepL", "deepLX"] as const)("keeps the complete %s route intersection searchable and matches settings language labels", async route => {
  configure({ profiles: [{ ...props.settings.profiles[0], textTranslation: route }] });
  await mount();
  await act(async () => picker(I18N.overlay.sourceLanguage).click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options.length).toBeGreaterThan(6);
  expect(options.map(option => option.textContent)).toEqual(sourceLanguagesForSettings(props.settings)
    .map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]));
  expect(document.querySelector("input.mimi-select__search")).not.toBeNull();
  const chinese = options.find(option => option.textContent === SOURCE_LANGUAGE_DISPLAY_NAMES.zh)!;
  await act(async () => chinese.click());
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith("zh");
});

it.each(["openAICompatible", "chatMock"] as const)("searches all 31 sources with the %s text route and sends the selected code", async route => {
  configure({ profiles: [{ ...props.settings.profiles[0], textTranslation: route }] });
  await mount();
  await act(async () => picker(I18N.overlay.sourceLanguage).click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options).toHaveLength(31);
  expect(options.map(option => option.textContent)).toEqual(["auto" as const, ...AUDIO3_RECOGNITION_LANGUAGE_CODES]
    .map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]));
  const search = document.querySelector<HTMLInputElement>("input.mimi-select__search")!;
  expect(search.getAttribute("aria-label")).toBe(I18N.settings.searchLanguages);
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "Norwegian");
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const filtered = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(filtered.map(option => option.textContent)).toEqual([SOURCE_LANGUAGE_DISPLAY_NAMES.no]);
  await act(async () => filtered[0].click());
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith("no");
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(document.querySelector('[role="listbox"]')).toBeNull();
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

async function chooseTarget(label: string) {
  await act(async () => picker(I18N.settings.translateTo).click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(choice => choice.textContent === label)!;
  await act(async () => option.click());
}

it("offers recognition-only as a target and lets the listener choose a translation again", async () => {
  await mount();
  await chooseTarget(TARGET_LANGUAGE_DISPLAY_NAMES.original);
  expect(props.onSwitchTargetLanguage).toHaveBeenCalledExactlyOnceWith("original");
  configure({ targetLanguage: "original" });
  await mount();
  expect(picker(I18N.settings.translateTo).textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.original);
  await chooseTarget(TARGET_LANGUAGE_DISPLAY_NAMES.zh);
  expect(props.onSwitchTargetLanguage).toHaveBeenLastCalledWith("zh");
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("locks language changes during reconnect and offers Original only on supported profiles", async () => {
  props.isChangingSession = true;
  await mount();
  expect(picker(I18N.settings.translateTo).getAttribute("aria-disabled")).toBe("true");

  props.isChangingSession = false;
  configure({ profiles: [{ id: "ali", provider: "openAIRealtime", name: "OpenAI", credentialState: "present" }] });
  await mount();
  await act(async () => picker(I18N.settings.translateTo).click());
  const options = [...document.querySelectorAll('[role="option"]')].map(option => option.textContent);
  expect(options.length).toBeGreaterThan(0);
  expect(options).not.toContain(TARGET_LANGUAGE_DISPLAY_NAMES.original);
});

it("keeps custom service-default semantics and scope visible in overlay controls", async () => {
  configure({ targetLanguage: "original", sourceLanguage: "auto", profiles: [{ ...props.settings.profiles[0], provider: "customDashScopeASR" }] });
  await mount();
  expect(picker(I18N.overlay.sourceLanguage).textContent).toContain(I18N.settings.recognitionServiceDefault);
  expect(host.querySelector('.recognition-language-notice')?.textContent).toBe(I18N.settings.recognitionCustomNotice);
  expect(host.textContent).toContain(I18N.settings.recognitionDashScopeParameter);
});


it("keeps the panel and current language after a rejected source switch and shows its safe reason", async () => {
  props.onSwitchSourceLanguage = vi.fn().mockRejectedValue("source_switch_unsupported");
  await mount();
  await searchSource("fr");
  await act(async () => document.querySelector<HTMLElement>('[role="option"]')!.click());
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith("fr");
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.languageSwitchUnsupported);
  expect(picker(I18N.overlay.sourceLanguage).textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.auto);
});

function configureProfiles() {
  configure({ profiles: [
    { id: "ali", name: "Alibaba Cloud", provider: "alibabaCloud", credentialState: "present" },
    { id: "custom", name: "My recognition model", provider: "openAIRealtime", credentialState: "present" },
  ], activeProfileId: "ali" });
}

async function chooseProfile(name: string) {
  await act(async () => picker(I18N.settings.currentProfile).click());
  const option = profileOption(name);
  await act(async () => option.click());
}

function profileOption(name: string) {
  return [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.querySelector(".mimi-select__label")?.textContent === name)!;
}

it.each([
  { credentialState: "missing", workspace: false },
  { credentialState: "unavailable", workspace: false },
  { credentialState: "present", workspace: true },
] as const)("explains and blocks an unavailable profile: $credentialState, workspace=$workspace", async ({ credentialState, workspace }) => {
  const unavailable = { id: "incomplete", name: "Incomplete service", provider: "alibabaCloud" as const, credentialState,
    alibabaRealtimeMode: workspace ? "liveTranslate" as const : "disabled" as const, alibabaWorkspaceId: "  " };
  configure({ profiles: [...props.settings.profiles, unavailable] });
  await mount();
  await act(async () => picker(I18N.settings.currentProfile).click());
  const option = profileOption(unavailable.name);
  expect(option.getAttribute("aria-disabled")).toBe("true");
  expect(option.querySelector(".mimi-select__description")?.textContent).toContain(workspace ? I18N.settings.appleSpeechNeedsSetup
    : credentialState === "missing" ? I18N.settings.credentialMissing : I18N.settings.credentialUnavailable);
  await act(async () => option.click());
  expect(document.querySelector('[role="listbox"]')).not.toBeNull();
  await key(picker(I18N.settings.currentProfile), "End");
  await key(picker(I18N.settings.currentProfile), "Enter");
  expect(props.onSelectProfile).not.toHaveBeenCalled();
  expect(picker(I18N.settings.currentProfile).textContent).toBe("Alibaba Cloud");
});

it("keeps an invalid current profile selected and explained, then restores it after setup", async () => {
  configureProfiles();
  configure({ profiles: props.settings.profiles.map(profile => profile.id === "ali" ? { ...profile, credentialState: "missing" } : profile) });
  await mount();
  await act(async () => picker(I18N.settings.currentProfile).click());
  const current = profileOption("Alibaba Cloud");
  expect(current.getAttribute("aria-selected")).toBe("true");
  expect(current.getAttribute("aria-disabled")).toBe("true");
  expect(current.querySelector(".mimi-select__description")?.textContent).toBe(I18N.settings.credentialMissing);
  await act(async () => current.click());
  expect(props.onSelectProfile).not.toHaveBeenCalled();
  await act(async () => profileOption("My recognition model").click());
  expect(props.onSelectProfile).toHaveBeenCalledExactlyOnceWith("custom");
  configure({ activeProfileId: "custom", profiles: props.settings.profiles.map(profile => ({ ...profile, credentialState: "present" })) });
  await mount();
  await act(async () => picker(I18N.settings.currentProfile).click());
  expect(profileOption("Alibaba Cloud").getAttribute("aria-disabled")).not.toBe("true");
  expect(profileOption("Alibaba Cloud").querySelector(".mimi-select__description")).toBeNull();
  await act(async () => profileOption("Alibaba Cloud").click());
  expect(props.onSelectProfile).toHaveBeenLastCalledWith("ali");
});

it.each(["appleSpeech", "customDashScopeASR", "customOpenAIASR"] as const)(
  "allows a %s Original preset but blocks its incomplete translated preset", async provider => {
    const original = { id: "original", name: "Recognition only", provider, credentialState: "missing" as const,
      speechCredentialState: "present" as const, textCredentialState: "missing" as const,
      languagePreset: { sourceLanguage: "en" as const, targetLanguage: "original" as const } };
    const translated = { ...original, id: "translated", name: "Translation needs setup",
      languagePreset: { sourceLanguage: "en" as const, targetLanguage: "zh" as const } };
    configure({ targetLanguage: "original", profiles: [...props.settings.profiles, original, translated] });
    await mount();
    await act(async () => picker(I18N.settings.currentProfile).click());
    const incomplete = profileOption(translated.name);
    expect(incomplete.getAttribute("aria-disabled")).toBe("true");
    expect(incomplete.querySelector(".mimi-select__description")?.textContent).toContain(profileLanguagePresetLabel(translated));
    await act(async () => incomplete.click());
    expect(props.onSelectProfile).not.toHaveBeenCalled();
    const available = profileOption(original.name);
    expect(available.getAttribute("aria-disabled")).not.toBe("true");
    expect(available.querySelector(".mimi-select__description")?.textContent).toBe(profileLanguagePresetLabel(original));
    await act(async () => available.click());
    expect(props.onSelectProfile).toHaveBeenCalledExactlyOnceWith(original.id);
  },
);

it.each([false, true])("switches saved profiles in the floating panel with paused=%s and keeps it open", async isPaused => {
  configureProfiles();
  props.isPaused = isPaused;
  await mount();
  expect(picker(I18N.settings.currentProfile).disabled).toBe(false);
  expect(picker(I18N.settings.currentProfile).querySelector(".provider-icon")?.getAttribute("data-provider")).toBe("alibabaCloud");
  await chooseProfile("Alibaba Cloud");
  expect(props.onSelectProfile).not.toHaveBeenCalled();
  await chooseProfile("My recognition model");
  expect(props.onSelectProfile).toHaveBeenCalledExactlyOnceWith("custom");
  expect(props.onDismiss).not.toHaveBeenCalled();
  configure({ activeProfileId: "custom" });
  await mount();
  expect(picker(I18N.settings.currentProfile).textContent).toBe("My recognition model");
  expect(picker(I18N.settings.currentProfile).querySelector(".provider-icon")?.getAttribute("data-provider")).toBe("openAIRealtime");
});

it("blocks profile changes during transitions, retains the saved profile on failure and supports sanitized retry", async () => {
  configureProfiles();
  props.isChangingSession = true;
  await mount();
  expect(picker(I18N.settings.currentProfile).getAttribute("aria-disabled")).toBe("true");
  props.isChangingSession = false;
  let reject!: (reason: Error) => void;
  props.onSelectProfile = vi.fn()
    .mockImplementationOnce(() => new Promise<void>((_resolve, failure) => { reject = failure; }))
    .mockResolvedValue(undefined);
  await mount();
  await act(async () => picker(I18N.settings.currentProfile).click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === "My recognition model")!;
  await act(async () => { option.click(); option.click(); });
  expect(props.onSelectProfile).toHaveBeenCalledExactlyOnceWith("custom");
  expect(picker(I18N.settings.currentProfile).getAttribute("aria-disabled")).toBe("true");
  expect(displayChoices().every(choice => choice.disabled)).toBe(true);
  expect(host.querySelector('.overlay-control-picker--profile')?.getAttribute("aria-busy")).toBe("true");
  await act(async () => reject(new Error("private-profile-switch-failure")));
  expect(picker(I18N.settings.currentProfile).textContent).toBe("Alibaba Cloud");
  expect(picker(I18N.settings.currentProfile).disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(profileErrorMessage("private-profile-switch-failure"));
  expect(host.textContent).not.toContain("private-profile-switch-failure");
  await chooseProfile("My recognition model");
  expect(props.onSelectProfile).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(props.onDismiss).not.toHaveBeenCalled();
});


it.each(["language_switch_superseded", "custom_speech_unreachable"])("keeps Original target failures visible for %s", async error => {
  props.onSwitchTargetLanguage = vi.fn().mockRejectedValue(error);
  await mount();
  await chooseTarget(TARGET_LANGUAGE_DISPLAY_NAMES.original);
  const target = picker(I18N.settings.translateTo);
  expect(props.onSwitchTargetLanguage).toHaveBeenCalledExactlyOnceWith("original");
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(target.textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.zh);
  expect(target.disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(error === "language_switch_superseded" ? I18N.settings.languageSwitchSuperseded : I18N.settings.customSpeechUnreachable);
  expect(host.textContent).not.toContain(error);
});

it("keeps the persisted profile visible when its reconnect fails", async () => {
  configureProfiles();
  const error = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
  let reject!: (reason: string) => void;
  props.onSelectProfile = vi.fn(() => new Promise<void>((_resolve, failure) => { reject = failure; }));
  await mount();
  await chooseProfile("My recognition model");
  configure({ activeProfileId: "custom" });
  await mount();
  await act(async () => reject(error));
  expect(picker(I18N.settings.currentProfile).textContent).toBe("My recognition model");
  expect(picker(I18N.settings.currentProfile).disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(profileErrorMessage(error));
  expect(host.textContent).not.toContain(error);
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(props.onSelectProfile).toHaveBeenCalledExactlyOnceWith("custom");
});


it.each(["zh", "en", "ja"] as const)("uses the expanded xAI sources and current target label in the %s floating controller", async locale => {
  setStoredUiLanguage(locale);
  configure({ sourceLanguage: "auto", targetLanguage: "pt-PT", profiles: [{ ...props.settings.profiles[0], provider: "xAIRealtime" }] });
  props.status = languageStatus(props.settings, null)!;
  await mount();
  expect(host.textContent).toContain(TARGET_LANGUAGE_DISPLAY_NAMES["pt-PT"]);
  await act(async () => picker(I18N.overlay.sourceLanguage).click());
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent))
    .toEqual(languageCatalogs.xAIRealtime.sourceLanguages.map(code => SOURCE_LANGUAGE_DISPLAY_NAMES[code as keyof typeof SOURCE_LANGUAGE_DISPLAY_NAMES]));
  const choice = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === SOURCE_LANGUAGE_DISPLAY_NAMES["pt-BR"])!;
  await act(async () => choice.click());
  expect(props.onSwitchSourceLanguage).toHaveBeenCalledExactlyOnceWith("pt-BR");
});

it("shows Gemini automatic source and an expanded target without inventing manual source choices", async () => {
  configure({ sourceLanguage: "auto", targetLanguage: "uk", profiles: [{ ...props.settings.profiles[0], provider: "googleGeminiLive" }] });
  props.status = languageStatus(props.settings, null)!;
  await mount();
  expect(picker(I18N.overlay.sourceLanguage).disabled).toBe(true);
  expect(picker(I18N.settings.translateTo).disabled).toBe(false);
  expect(host.textContent).toContain(TARGET_LANGUAGE_DISPLAY_NAMES.uk);
  expect(host.querySelector(".overlay-control-header__collapse")?.getAttribute("aria-label")).toContain(I18N.overlay.autoDetecting);
});

it.each(["zh", "en", "ja"] as const)("offers every Gemini target beside the disabled automatic source in %s", async locale => {
  setStoredUiLanguage(locale);
  configure({ sourceLanguage: "auto", targetLanguage: "zh", profiles: [{ ...props.settings.profiles[0], provider: "googleGeminiLive" }] });
  await mount();
  expect(picker(I18N.overlay.sourceLanguage).disabled).toBe(true);
  const target = picker(I18N.settings.translateTo);
  await act(async () => target.click());
  const choices = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(choices.map(choice => choice.textContent)).toEqual(languageCatalogs.googleGeminiLive.targetLanguages.map(code => TARGET_LANGUAGE_DISPLAY_NAMES[code as keyof typeof TARGET_LANGUAGE_DISPLAY_NAMES]));
  await act(async () => choices.find(choice => choice.textContent === TARGET_LANGUAGE_DISPLAY_NAMES["pt-BR"])!.click());
  expect(props.onSwitchTargetLanguage).toHaveBeenCalledExactlyOnceWith("pt-BR");
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("keeps the saved target after a failed switch and permits retry without dismissing the panel", async () => {
  let reject!: (error: unknown) => void;
  props.onSwitchTargetLanguage = vi.fn().mockImplementationOnce(() => new Promise<void>((_, fail) => { reject = fail; })).mockResolvedValue(undefined);
  configure({ sourceLanguage: "auto", targetLanguage: "zh", profiles: [{ ...props.settings.profiles[0], provider: "googleGeminiLive" }] });
  await mount();
  const target = picker(I18N.settings.translateTo);
  const selectFrench = () => act(() => [...document.querySelectorAll<HTMLElement>('[role="option"]')]
    .find(choice => choice.textContent === TARGET_LANGUAGE_DISPLAY_NAMES.fr)!.click());
  await act(() => target.click());
  await selectFrench();
  expect(target.getAttribute("aria-disabled")).toBe("true");
  expect(picker(I18N.settings.currentProfile).getAttribute("aria-disabled")).toBe("true");
  expect(document.activeElement).toBe(target);
  await act(async () => reject("target_switch_unsupported"));
  expect(target.textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.zh);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.languageSwitchUnsupported);
  await act(() => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
  expect(target.getAttribute("aria-expanded")).toBe("true");
  await selectFrench();
  expect(props.onSwitchTargetLanguage).toHaveBeenCalledTimes(2);
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it.each(["zh", "en", "ja"] as const)("keeps duplicate toolbar actions out of the %s configuration panel", async language => {
  setStoredUiLanguage(language);
  await mount();
  expect(host.querySelector(".overlay-control-session-action")).toBeNull();
  expect(host.querySelector(`button[aria-label="${I18N.overlay.pause}"]`)).toBeNull();
  expect(host.querySelector(`button[aria-label="${I18N.overlay.closeSubtitles}"]`)).toBeNull();
  expect(host.querySelector(`button[aria-label="${I18N.overlay.immersiveMode}"]`)).toBeNull();
  expect(host.querySelector(`button[aria-label="${I18N.overlay.lockPosition}"]`)).toBeNull();
});
