// @vitest-environment jsdom
import { act, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../../lib/i18n";
import { useStore } from "../../../lib/store";
import type { SettingsSnapshot } from "../../../lib/types";
import { SettingsToastRegion } from "../SettingsToast";
import { useLanguageNormalizationToast } from "./useLanguageNormalizationToast";
let host: HTMLDivElement, root: Root;
let track: ReturnType<typeof useLanguageNormalizationToast>;
const before: SettingsSnapshot = { ...useStore.getState().settings, sourceLanguage: "fr", targetLanguage: "original", languageCapabilities: undefined,
  activeProfileId: "custom", profiles: [{ id: "custom", name: "Custom", provider: "customDashScopeASR", credentialState: "present", textTranslation: "deepL" }] };
const after: SettingsSnapshot = { ...before, targetLanguage: "zh", sourceLanguage: "auto" };
function Harness({ settings }: { settings: SettingsSnapshot }) { const handler = useLanguageNormalizationToast(settings); useEffect(() => { track = handler; }, [handler]); return <SettingsToastRegion />; }
const render = async (settings: SettingsSnapshot) => { await act(async () => root.render(<Harness settings={settings} />)); };
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); setStoredUiLanguage("en"); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });
it.each([true, false])("reports the actual normalized source when readback-first is %s", async readbackFirst => {
  await render(before);
  const finish = track("zh");
  if (readbackFirst) await render(after);
  else await act(async () => finish(true));
  expect(host.querySelector('[role="status"]')).toBeNull();
  if (readbackFirst) await act(async () => finish(true));
  else await render(after);
  expect(host.textContent).toContain(I18N.settings.recognitionLanguageAdjusted("French", I18N.settings.recognitionServiceDefault));
});
it("does not announce a rejected or blurred operation as a successful language change", async () => {
  await render(before);
  const failed = track("zh"); failed(false);
  await render(after);
  expect(host.querySelector('[role="status"]')).toBeNull();
  await render(before);
  const blurred = track("zh");
  await act(async () => window.dispatchEvent(new Event("blur")));
  await act(async () => blurred(true));
  await render(after);
  expect(host.querySelector('[role="status"]')).toBeNull();
});
