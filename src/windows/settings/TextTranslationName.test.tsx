// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N } from "../../lib/i18n";
import type { ServiceProfile, TextTranslation } from "../../lib/types";
import { TextTranslationName } from "./TextTranslationName";

const profile: ServiceProfile = { id: "synthetic-profile", name: "Configuration", provider: "customOpenAIASR", credentialState: "present", textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: "My translator", deepL: "My DeepL" } };
let host: HTMLDivElement, root: Root;
const save = vi.fn();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.useFakeTimers();
  save.mockReset().mockResolvedValue({});
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
async function render(route: Exclude<TextTranslation, "followService"> = "openAICompatible", current = profile) {
  await act(() => root.render(<TextTranslationName profile={current} route={route} inputId="translation-name" disabled={false} onSave={save} />));
}
const input = () => host.querySelector<HTMLInputElement>("#translation-name")!;
async function change(value: string) {
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function tick() { await act(async () => vi.advanceTimersByTimeAsync(350)); }

it("auto-saves route metadata without a Save button and preserves the raw draft after acknowledgement", async () => {
  await render();
  expect(input().value).toBe("My translator");
  expect(input().placeholder).toBe(I18N.settings.textTranslationOpenAICompatible);
  expect(input().maxLength).toBe(64);
  expect(host.querySelector('.settings-button')).toBeNull();
  await change("  Office translator  "); await tick();
  expect(save).toHaveBeenCalledExactlyOnceWith("openAICompatible", "Office translator");
  await render("openAICompatible", { ...profile, textTranslationNames: { openAICompatible: "Office translator" } });
  expect(input().value).toBe("  Office translator  ");
  await change("B 站"); await tick();
  expect(save).toHaveBeenLastCalledWith("openAICompatible", "B 站");
  await change(""); await tick();
  expect(save).toHaveBeenLastCalledWith("openAICompatible", "");
  expect(input().value).toBe("");
});

it("flushes the old route and profile draft to its own identity when switching editors", async () => {
  await render(); await change("Old route final draft");
  await render("deepL");
  expect(save).toHaveBeenCalledExactlyOnceWith("openAICompatible", "Old route final draft");
  expect(input().value).toBe("My DeepL");
  await change("Old profile final draft");
  await render("deepL", { ...profile, id: "second-profile", textTranslationNames: {} });
  expect(save).toHaveBeenLastCalledWith("deepL", "Old profile final draft");
  expect(input().value).toBe("");
});
