// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { profileErrorMessage } from "../../lib/connectionDiagnostics";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import type { ServiceProfile, TextTranslation } from "../../lib/types";
import { TextTranslationName } from "./TextTranslationName";

type IndependentRoute = Exclude<TextTranslation, "followService">;
const profile: ServiceProfile = {
  id: "synthetic-profile",
  name: "Synthetic configuration",
  provider: "customOpenAIASR",
  credentialState: "present",
  textTranslation: "openAICompatible",
  textTranslationNames: { openAICompatible: "My translator", deepL: "My DeepL" },
};
let host: HTMLDivElement, root: Root;
let save: ReturnType<typeof vi.fn<(route: IndependentRoute, name: string) => Promise<unknown>>>;
let credentialsSubmit: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  save = vi.fn().mockResolvedValue({});
  credentialsSubmit = vi.fn();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(() => root.unmount());
  host.remove();
  setStoredUiLanguage("system");
  vi.unstubAllGlobals();
});

async function render({ route = "openAICompatible", current = profile, disabled = false }: {
  route?: IndependentRoute;
  current?: ServiceProfile;
  disabled?: boolean;
} = {}) {
  await act(() => root.render(
    <form onSubmit={event => { event.preventDefault(); credentialsSubmit(); }}>
      <TextTranslationName key={`${current.id}:${route}`} profile={current} route={route}
        inputId="translation-name" disabled={disabled} onSave={save} />
      <button type="submit">Save credentials</button>
    </form>,
  ));
}

const input = () => host.querySelector<HTMLInputElement>("#translation-name")!;
const saveButton = () => [...host.querySelectorAll<HTMLButtonElement>("button")]
  .find(button => button.textContent === I18N.settings.saveName);

async function change(value: string) {
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it("shows the saved route name with the protocol as its default and a bounded field", async () => {
  await render();
  expect(input().value).toBe("My translator");
  expect(input().placeholder).toBe(I18N.settings.textTranslationOpenAICompatible);
  expect(input().maxLength).toBe(64);
  expect(host.textContent).toContain(I18N.settings.textTranslationName);
  expect(saveButton()).toBeUndefined();
  await change("My translator  ");
  expect(saveButton()).toBeUndefined();
  expect(save).not.toHaveBeenCalled();
});

it("saves a trimmed name without submitting or changing the credential form", async () => {
  await render();
  await change("  Office translator  ");
  expect(saveButton()?.type).toBe("button");
  await act(async () => saveButton()!.click());
  expect(save).toHaveBeenCalledExactlyOnceWith("openAICompatible", "Office translator");
  expect(credentialsSubmit).not.toHaveBeenCalled();
  expect(input().value).toBe("Office translator");
  expect(saveButton()).toBeUndefined();
  expect(host.querySelector('[role="status"]')).toBeNull();
});

it("uses Enter for the metadata save and prevents the enclosing credential submit", async () => {
  await render();
  await change("Local model");
  const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
  await act(async () => { input().dispatchEvent(event); });
  expect(event.defaultPrevented).toBe(true);
  expect(save).toHaveBeenCalledExactlyOnceWith("openAICompatible", "Local model");
  expect(credentialsSubmit).not.toHaveBeenCalled();
});

it("does not save the name while Enter is confirming an IME composition", async () => {
  await render();
  await change("我的翻译");
  const event = new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true });
  await act(() => { input().dispatchEvent(event); });
  expect(save).not.toHaveBeenCalled();
  expect(input().value).toBe("我的翻译");
  expect(saveButton()).toBeDefined();
});

it("clears an existing name with an empty value so the default can be restored", async () => {
  await render();
  await change("   ");
  await act(async () => saveButton()!.click());
  expect(save).toHaveBeenCalledExactlyOnceWith("openAICompatible", "");
  expect(input().value).toBe("");
  expect(input().placeholder).toBe(I18N.settings.textTranslationOpenAICompatible);
  expect(saveButton()).toBeUndefined();
});

it("retains the unsaved name and permits retry when the parent rejects the save with null", async () => {
  save.mockResolvedValueOnce(null);
  await render();
  await change("Unsaved translator");
  await act(async () => saveButton()!.click());
  expect(input().value).toBe("Unsaved translator");
  expect(saveButton()?.disabled).toBe(false);
  expect(host.querySelector('[role="status"]')).toBeNull();
  await act(async () => saveButton()!.click());
  expect(save).toHaveBeenCalledTimes(2);
  expect(saveButton()).toBeUndefined();
});

it("keeps a failed draft and an actionable sanitized error beside the field", async () => {
  const error = new Error("synthetic-private-storage-path");
  save.mockRejectedValueOnce(error);
  await render();
  await change("Unsaved translator");
  await act(async () => saveButton()!.click());
  expect(input().value).toBe("Unsaved translator");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(profileErrorMessage(error));
  expect(host.textContent).not.toContain("synthetic-private-storage-path");
  expect(saveButton()?.disabled).toBe(false);
  await act(async () => saveButton()!.click());
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(saveButton()).toBeUndefined();
});

it("guards disabled and duplicate save actions while a name is being persisted", async () => {
  await render();
  await change("New translator");
  await render({ disabled: true });
  expect(input().disabled).toBe(true);
  expect(saveButton()?.disabled).toBe(true);
  await act(async () => saveButton()!.click());
  expect(save).not.toHaveBeenCalled();

  let finish!: (result: unknown) => void;
  save.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await render();
  await act(() => { saveButton()!.click(); saveButton()!.click(); });
  expect(save).toHaveBeenCalledExactlyOnceWith("openAICompatible", "New translator");
  expect(input().disabled).toBe(true);
  expect(saveButton()?.disabled).toBe(true);
  await act(async () => finish({}));
  expect(input().disabled).toBe(false);
  expect(saveButton()).toBeUndefined();
});

it("separates route and profile drafts and ignores completion from a remounted editor", async () => {
  let finish!: (result: unknown) => void;
  save.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await render();
  await change("Pending old route");
  await act(() => saveButton()!.click());
  await render({ route: "deepL" });
  expect(input().value).toBe("My DeepL");
  expect(input().placeholder).toBe("DeepL");
  await act(async () => finish({}));
  expect(input().value).toBe("My DeepL");
  expect(saveButton()).toBeUndefined();

  await change("Unsaved DeepL");
  await render({ route: "deepL", current: { ...profile, id: "second-profile", textTranslationNames: {} } });
  expect(input().value).toBe("");
  expect(saveButton()).toBeUndefined();
});

it("preserves edits across unrelated snapshots and refreshes when the saved name changes", async () => {
  await render();
  await change("Unsaved name");
  await render({ current: { ...profile, credentialState: "missing" } });
  expect(input().value).toBe("Unsaved name");
  expect(saveButton()).toBeDefined();
  await render({ current: { ...profile, textTranslationNames: { openAICompatible: "Saved elsewhere" } } });
  expect(input().value).toBe("Saved elsewhere");
  expect(saveButton()).toBeUndefined();
  expect(save).not.toHaveBeenCalled();
});

it("expands a long saved name for editing and keeps Enter scoped to the name save", async () => {
  const name = "翻译服务名称".repeat(8);
  await render({ current: { ...profile, textTranslationNames: { openAICompatible: name } } });
  await act(() => host.querySelector<HTMLButtonElement>('.config-input__expand')!.click());
  const expanded = host.querySelector<HTMLTextAreaElement>('#translation-name')!;
  expect(expanded.tagName).toBe("TEXTAREA");
  expect(expanded.value).toBe(name);
  expect(expanded.maxLength).toBe(64);
  expect(save).not.toHaveBeenCalled();
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(expanded, "我的翻译");
    expanded.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => expanded.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
  expect(save).toHaveBeenCalledExactlyOnceWith("openAICompatible", "我的翻译");
  expect(credentialsSubmit).not.toHaveBeenCalled();
});
