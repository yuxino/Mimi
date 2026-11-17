// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import languageCatalogs from "../../../shared/provider-language-catalogs.json";
import { languageStatus } from "../overlay/overlayModel";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import { useStore } from "../../lib/store";
import { OverlayControlPanel } from "./OverlayControlPanel";
import { overlayControlPanelModel } from "./overlayControlModel";
import { sourceLanguagesForSettings } from "../../lib/providerCapabilities";
import { profileLanguagePresetLabel } from "../../lib/profileLanguagePreset";
import { SUBTITLE_DISPLAY_OPTIONS } from "../../lib/subtitleDisplay";
import { AUDIO3_RECOGNITION_LANGUAGE_CODES, SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type SettingsSnapshot } from "../../lib/types";

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
    isPaused: false, isWaitingForFinalTranslation: false, isChangingSession: false, hasContent: false,
    onDismiss: vi.fn(), onSwitchSourceLanguage: vi.fn().mockResolvedValue(undefined),
    onSwitchTargetLanguage: vi.fn().mockResolvedValue(undefined),
    onSelectProfile: vi.fn().mockResolvedValue(undefined),
    onSetSubtitleDisplayMode: vi.fn().mockResolvedValue(undefined), onSetImmersiveMode: vi.fn().mockResolvedValue(undefined),
    onSetOverlayLocked: vi.fn().mockResolvedValue(undefined), onClearSubtitles: vi.fn().mockResolvedValue(undefined),
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

it.each(["zh", "en", "ja"] as const)("groups %s language, audio and subtitle controls in reading order", async (language) => {
  setStoredUiLanguage(language);
  await mount();
  const [languageSection, audioSection, subtitleSection] = host.querySelectorAll("section.overlay-control-section");
  expect(languageSection.getAttribute("aria-label")).toBe(I18N.settings.voiceTranslation);
  expect(languageSection.contains(picker(I18N.overlay.sourceLanguage))).toBe(true);
  expect(audioSection.querySelector("h2")?.textContent).toBe(I18N.settings.audioInputTitle);
  expect(subtitleSection.querySelector("h2")?.textContent).toBe(I18N.settings.subtitleTitle);
  expect(host.querySelectorAll('[role="combobox"]')).toHaveLength(4);
  expect(displayChoices().map(choice => choice.textContent)).toEqual(SUBTITLE_DISPLAY_OPTIONS.map(option => option.label));
  expect(host.querySelectorAll('.application-audio-picker')).toHaveLength(1);
  expect(host.querySelector('fieldset, .overlay-control-options, .overlay-control-group')).toBeNull();
  expect(host.querySelectorAll('[role="switch"]')).toHaveLength(2);
  expect(host.querySelector('.overlay-control-setting small')).toBeNull();
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
  await act(async () => host.querySelector<HTMLButtonElement>(".overlay-control-settings-link")!.click());
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

it("clears only available subtitles, blocks concurrent actions, and keeps a sanitized retry in the panel", async () => {
  let reject!: (error: Error) => void;
  props.onClearSubtitles = vi.fn()
    .mockImplementationOnce(() => new Promise<void>((_resolve, failure) => { reject = failure; }))
    .mockResolvedValue(undefined);
  await mount();
  const clear = host.querySelector<HTMLButtonElement>(".overlay-control-clear")!;
  expect(clear.disabled).toBe(true);
  await act(async () => clear.click());
  expect(props.onClearSubtitles).not.toHaveBeenCalled();

  props.hasContent = true;
  await mount();
  const bilingual = displayChoices().find(choice => choice.textContent === I18N.settings.displayBilingual)!;
  await act(async () => { clear.click(); clear.click(); bilingual.click(); });
  expect(props.onClearSubtitles).toHaveBeenCalledOnce();
  expect(props.onSetSubtitleDisplayMode).not.toHaveBeenCalled();
  expect(clear.disabled).toBe(true);
  expect(displayChoices().every(choice => choice.disabled)).toBe(true);

  await act(async () => reject(new Error("private-clear-failure")));
  expect(host.querySelector('.settings-toast[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  expect(host.textContent).not.toContain("private-clear-failure");
  expect(clear.disabled).toBe(false);
  await act(async () => clear.click());
  expect(props.onClearSubtitles).toHaveBeenCalledTimes(2);
  expect(host.querySelector(".settings-toast")).toBeNull();
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it("blocks conflicting operations and retains an inline error after a failed toggle", async () => {
  let reject!: (reason: Error) => void;
  props.onSetImmersiveMode = vi.fn(() => new Promise<void>((_resolve, failure) => { reject = failure; }));
  await mount();
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.overlay.immersiveMode}"]`)!.click());
  expect([...host.querySelectorAll<HTMLButtonElement>('[role="combobox"], [role="switch"]'), ...displayChoices()].every((button) => button.disabled)).toBe(true);
  await act(async () => reject(new Error("synthetic-toggle-failure")));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.overlay.controlActionFailed);
  expect(props.onDismiss).not.toHaveBeenCalled();
});

it.each(["immersive", "lock"] as const)("does not dismiss a reopened panel when an old %s save completes", async action => {
  let finish!: () => void;
  const operation = vi.fn(() => new Promise<void>(resolve => { finish = resolve; }));
  if (action === "immersive") props.onSetImmersiveMode = operation;
  else props.onSetOverlayLocked = operation;
  await mount();
  const label = action === "immersive" ? I18N.overlay.immersiveMode : I18N.overlay.lockPosition;
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${label}"]`)!.click());
  expect(operation).toHaveBeenCalledOnce();
  await act(async () => root.render(null));
  await mount();
  await act(async () => finish());
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(host.querySelector('[role="dialog"]')).not.toBeNull();
});

it.each([false, true])("preserves saved reading mode %j while error presentation overrides lock and immersive actions", async enabled => {
  configure({ subtitleBlendsWithBackground: enabled, isOverlayLocked: enabled });
  const savedSettings = props.settings;
  const actions = () => [I18N.overlay.immersiveMode, I18N.overlay.lockPosition].map(label =>
    host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${label}"]`)!,
  );
  await mount();
  expect(actions().every(button => !button.disabled)).toBe(true);
  props.sessionErrorMessage = I18N.settings.sessionError;
  await mount();
  for (const button of actions()) {
    expect(button.disabled).toBe(true);
    expect(button.getAttribute("aria-checked")).toBe(String(enabled));
    await act(async () => button.click());
    expect(button.disabled).toBe(true);
  }
  expect(props.onSetImmersiveMode).not.toHaveBeenCalled();
  expect(props.onSetOverlayLocked).not.toHaveBeenCalled();
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(displayChoices().every(choice => !choice.disabled)).toBe(true);
  expect(host.querySelector<HTMLButtonElement>(".overlay-control-settings-link")!.disabled).toBe(false);
  props.sessionErrorMessage = null;
  await mount();
  expect(props.settings).toBe(savedSettings);
  for (const button of actions()) {
    expect(button.disabled).toBe(false);
    expect(button.getAttribute("aria-checked")).toBe(String(enabled));
    await act(async () => button.click());
  }
  expect(props.onSetImmersiveMode).toHaveBeenCalledExactlyOnceWith(!enabled);
  expect(props.onSetOverlayLocked).toHaveBeenCalledExactlyOnceWith(!enabled);
  expect(props.onDismiss).toHaveBeenCalledTimes(2);
});

it("keeps recognition locked during transitions while independent display remains available", async () => {
  props.isChangingSession = true;
  await mount();
  expect(picker(I18N.overlay.sourceLanguage).disabled).toBe(true);
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
  expect(picker(I18N.settings.translateTo).disabled).toBe(true);

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
  expect(picker(I18N.settings.currentProfile).disabled).toBe(true);
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
  expect(picker(I18N.settings.currentProfile).disabled).toBe(true);
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
  expect(host.querySelector(".overlay-control-header")?.getAttribute("aria-label")).toContain(I18N.overlay.autoDetecting);
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
  const change = async () => {
    await act(async () => picker(I18N.settings.translateTo).click());
    await act(async () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(choice => choice.textContent === TARGET_LANGUAGE_DISPLAY_NAMES.fr)!.click());
  };
  await change();
  expect(picker(I18N.settings.translateTo).disabled).toBe(true);
  expect(picker(I18N.settings.currentProfile).disabled).toBe(true);
  await act(async () => reject("target_switch_unsupported"));
  expect(picker(I18N.settings.translateTo).textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.zh);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.languageSwitchUnsupported);
  await change();
  expect(props.onSwitchTargetLanguage).toHaveBeenCalledTimes(2);
  expect(props.onDismiss).not.toHaveBeenCalled();
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it.each(["zh", "en", "ja"] as const)("keeps duplicate pause/close actions out of the %s configuration panel", async language => {
  setStoredUiLanguage(language);
  await mount();
  expect(host.querySelector(".overlay-control-session-action")).toBeNull();
  expect(host.querySelector(`button[aria-label="${I18N.overlay.pause}"]`)).toBeNull();
  expect(host.querySelector(`button[aria-label="${I18N.overlay.closeSubtitles}"]`)).toBeNull();
});
