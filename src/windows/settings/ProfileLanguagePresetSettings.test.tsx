// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { ProfileLanguagePresetSettings } from "./ProfileLanguagePresetSettings";
import { SettingsToastRegion } from "./SettingsToast";

let host: HTMLDivElement, root: Root;
let props: ComponentProps<typeof ProfileLanguagePresetSettings>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  const settings = { ...useStore.getState().settings, sourceLanguage: "ja" as const, targetLanguage: "zh" as const };
  const profile = { id: "saved", name: "Alibaba", provider: "alibabaCloud" as const, credentialState: "present" as const };
  props = { profile, settings: { ...settings, profiles: [profile], activeProfileId: profile.id }, disabled: false,
    onSave: vi.fn().mockImplementation(async languagePreset => ({ ...props.settings, profiles: [{ ...props.profile, languagePreset }] })) };
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("en"); vi.unstubAllGlobals(); });
const render = async () => { await act(async () => root.render(<><ProfileLanguagePresetSettings {...props} /><SettingsToastRegion /></>)); };
const button = (label: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === label)!;

it.each(["en", "zh", "ja"] as const)("remembers the current pair in one action without extra language pickers (%s)", async language => {
  setStoredUiLanguage(language);
  await render();
  expect(host.querySelector('[role="combobox"]')).toBeNull();
  await act(async () => button(I18N.settings.profileLanguagesRemember).click());
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "ja", targetLanguage: "zh" });
  expect(props.settings.sourceLanguage).toBe("ja");
  expect(host.querySelector('[role="status"]')?.textContent).toContain(I18N.settings.profileLanguagesSaved);
});

it("keeps a saved pair across temporary changes until an explicit update", async () => {
  props.profile = { ...props.profile, languagePreset: { sourceLanguage: "en", targetLanguage: "zh" } };
  await render();
  expect(props.onSave).not.toHaveBeenCalled();
  await act(async () => button(I18N.settings.profileLanguagesUpdate).click());
  expect(props.onSave).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "ja", targetLanguage: "zh" });
});

it("retains the failed request for retry and removes the pair explicitly", async () => {
  props.profile = { ...props.profile, languagePreset: { sourceLanguage: "en", targetLanguage: "zh" } };
  await render();
  vi.mocked(props.onSave).mockRejectedValueOnce(new Error("private-error"));
  await act(async () => button(I18N.settings.profileLanguagesClear).click());
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.settings.profileLanguagesSaveFailed);
  expect(host.textContent).not.toContain("private-error");
  await act(async () => button(I18N.settings.retryLoadingSettings).click());
  expect(props.onSave).toHaveBeenLastCalledWith(null);
  expect(host.querySelector('[role="status"]')?.textContent).toContain(I18N.settings.profileLanguagesCleared);
});

it("does not bind an inactive configuration to another configuration's current languages", async () => {
  props.settings = { ...props.settings, activeProfileId: "other" };
  await render();
  expect(button(I18N.settings.profileLanguagesRemember)).toBeUndefined();
  expect(host.textContent).toContain(I18N.settings.profileLanguagesKeep);
  expect(props.onSave).not.toHaveBeenCalled();
});

it("locks duplicate saves and retains retry when a save is not acknowledged", async () => {
  await render();
  let finish!: (result: typeof props.settings) => void;
  vi.mocked(props.onSave).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await act(async () => button(I18N.settings.profileLanguagesRemember).click());
  expect(button(I18N.settings.profileLanguagesRemember).disabled).toBe(true);
  await act(async () => finish(props.settings));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.settings.profileLanguagesSaveFailed);
  expect(button(I18N.settings.profileLanguagesRemember).disabled).toBe(false);
});

it("does not revive a late result after leaving the editor", async () => {
  await render();
  let finish!: (result: typeof props.settings) => void;
  vi.mocked(props.onSave).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await act(async () => button(I18N.settings.profileLanguagesRemember).click());
  await act(async () => root.render(<SettingsToastRegion />));
  await act(async () => finish({ ...props.settings, profiles: [{ ...props.profile, languagePreset: { sourceLanguage: "ja", targetLanguage: "zh" } }] }));
  expect(host.textContent).toBe("");
});
