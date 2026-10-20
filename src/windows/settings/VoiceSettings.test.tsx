// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SettingsSnapshot } from "../../lib/types";
import { I18N } from "../../lib/i18n";
import { profileLanguagePresetLabel } from "../../lib/profileLanguagePreset";
import { VoiceSettings } from "./VoiceSettings";

const state = vi.hoisted(() => ({
  settings: {} as SettingsSnapshot,
  session: { status: { kind: "idle" } },
  selectProfile: vi.fn().mockResolvedValue({}),
}));
vi.mock("../../lib/store", async original => {
  const store = await original<typeof import("../../lib/store")>();
  state.settings = store.useStore.getState().settings;
  return { useStore: (selector: (value: typeof state) => unknown) => selector(state) };
});
vi.mock("./AudioInputSettings", () => ({ AudioInputSettings: () => null }));
vi.mock("./ProfileLanguageSettings", () => ({ ProfileLanguageSettings: () => null }));

let host: HTMLDivElement;
let root: Root;
beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  state.session.status.kind = "idle";
  state.selectProfile.mockClear();
  state.settings = { ...state.settings, activeProfileId: "ready", targetLanguage: "zh", profiles: [
    { id: "ready", name: "Ready service", provider: "alibabaCloud", credentialState: "present", alibabaRealtimeMode: "disabled" },
    { id: "missing", name: "Missing credentials", provider: "alibabaCloud", credentialState: "missing", alibabaRealtimeMode: "disabled" },
    { id: "unavailable", name: "Unavailable credentials", provider: "alibabaCloud", credentialState: "unavailable", alibabaRealtimeMode: "disabled" },
    { id: "workspace", name: "Missing workspace", provider: "alibabaCloud", credentialState: "present", alibabaRealtimeMode: "liveTranslate", alibabaWorkspaceId: "" },
    { id: "another", name: "Another ready service", provider: "alibabaCloud", credentialState: "present", alibabaRealtimeMode: "disabled" },
  ] };
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await render();
  await act(() => host.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
async function render() { await act(() => root.render(<VoiceSettings onOpenModels={() => {}} onOpenAppleResources={() => {}} />)); }
function option(label: string) {
  return [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.querySelector(".mimi-select__label")?.textContent === label)!;
}

it.each(["Missing credentials", "Unavailable credentials", "Missing workspace"])("keeps %s visible without dispatching a switch", async label => {
  const choice = option(label);
  const reason = label === "Missing credentials" ? I18N.settings.credentialMissing
    : label === "Unavailable credentials" ? I18N.settings.credentialUnavailable : I18N.settings.appleSpeechNeedsSetup;
  expect(choice.querySelector(".mimi-select__description")?.textContent).toContain(reason);
  await act(async () => choice.click());
  expect(state.selectProfile).not.toHaveBeenCalled();
  expect(choice.getAttribute("aria-disabled")).toBe("true");
  expect(document.querySelector('[role="listbox"]')).not.toBeNull();
});

it("keeps a configured service selectable", async () => {
  await act(async () => option("Another ready service").click());
  expect(state.selectProfile).toHaveBeenCalledExactlyOnceWith("another");
});

it("keeps the active selection visible when its credentials become missing", async () => {
  state.settings = { ...state.settings, profiles: state.settings.profiles.map(profile => profile.id === "ready" ? { ...profile, credentialState: "missing" } : profile) };
  await render();
  const active = option("Ready service");
  expect(active.getAttribute("aria-selected")).toBe("true");
  expect(active.getAttribute("aria-disabled")).toBe("true");
  expect(active.querySelector(".mimi-select__description")?.textContent).toBe(I18N.settings.credentialMissing);
  await act(async () => active.click());
  expect(state.selectProfile).not.toHaveBeenCalled();
  await act(async () => option("Another ready service").click());
  expect(state.selectProfile).toHaveBeenCalledExactlyOnceWith("another");
});

it("allows a saved configuration after its missing credentials become available", async () => {
  state.settings = { ...state.settings, profiles: state.settings.profiles.map(profile => profile.id === "missing" ? { ...profile, credentialState: "present" } : profile) };
  await render();
  expect(option("Missing credentials").getAttribute("aria-disabled")).not.toBe("true");
  expect(option("Missing credentials").querySelector(".mimi-select__description")).toBeNull();
  await act(async () => option("Missing credentials").click());
  expect(state.selectProfile).toHaveBeenCalledExactlyOnceWith("missing");
});

it("skips unavailable profiles with the keyboard while leaving them visible", async () => {
  state.settings = { ...state.settings, profiles: state.settings.profiles.filter(profile => profile.id !== "another") };
  await render();
  const picker = host.querySelector<HTMLButtonElement>('[role="combobox"]')!;
  for (const key of ["End", "Enter"]) {
    await act(async () => picker.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
  }
  expect(state.selectProfile).not.toHaveBeenCalled();
  expect(picker.textContent).toBe("Ready service");
});

it.each(["appleSpeech", "customDashScopeASR", "customOpenAIASR"] as const)(
  "keeps %s recognition-only presets selectable without a translation key", async provider => {
    const profile = { id: "original", name: "Recognition only", provider, credentialState: "missing" as const,
      speechCredentialState: "present" as const, textCredentialState: "missing" as const,
      languagePreset: { sourceLanguage: "en" as const, targetLanguage: "original" as const } };
    state.settings = { ...state.settings, profiles: [...state.settings.profiles, profile] };
    await render();
    const choice = option(profile.name);
    expect(choice.getAttribute("aria-disabled")).not.toBe("true");
    expect(choice.querySelector(".mimi-select__description")?.textContent).toBe(profileLanguagePresetLabel(profile));
    await act(async () => choice.click());
    expect(state.selectProfile).toHaveBeenCalledExactlyOnceWith(profile.id);
  },
);

it("checks a translated preset even while the current session shows only original text", async () => {
  const profile = { id: "translated", name: "Translation needs setup", provider: "customOpenAIASR" as const,
    credentialState: "missing" as const, speechCredentialState: "present" as const,
    languagePreset: { sourceLanguage: "en" as const, targetLanguage: "zh" as const } };
  state.settings = { ...state.settings, targetLanguage: "original", profiles: [...state.settings.profiles, profile] };
  await render();
  const choice = option(profile.name);
  expect(choice.getAttribute("aria-disabled")).toBe("true");
  expect(choice.querySelector(".mimi-select__description")?.textContent).toContain(I18N.settings.credentialMissing);
  expect(choice.querySelector(".mimi-select__description")?.textContent).toContain(profileLanguagePresetLabel(profile));
  await act(async () => choice.click());
  expect(state.selectProfile).not.toHaveBeenCalled();
});

it("does not revive the profile picker after native connecting and active state changes", async () => {
  state.session.status.kind = "connecting";
  await render();
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  state.session.status.kind = "active";
  await render();
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(state.selectProfile).not.toHaveBeenCalled();
  await act(() => host.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
  await act(async () => option("Another ready service").click());
  expect(state.selectProfile).toHaveBeenCalledExactlyOnceWith("another");
});
