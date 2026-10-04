// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { SettingsRow } from "./SettingsPrimitives";

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  setStoredUiLanguage("en");
  host = document.createElement("div"); document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(() => root.unmount()); host.remove();
  setStoredUiLanguage("en"); vi.restoreAllMocks(); vi.unstubAllGlobals();
});

async function render(description?: string, hint?: string) {
  await act(() => root.render(<SettingsRow label="Appearance" description={description} hint={hint}><button type="button">Change</button></SettingsRow>));
}

it("combines a description and hint into one shared help icon without persistent small print", async () => {
  await render("Choose how subtitles look.", "You can change this while subtitles are running.");
  expect(host.querySelector(".settings-row__label")?.textContent).toContain("Appearance");
  expect(host.querySelector(".settings-row__control button")?.textContent).toBe("Change");
  expect(host.querySelectorAll(".settings-help-control")).toHaveLength(1);
  expect(host.querySelector(".settings-help-control__description")?.textContent).toBe("Choose how subtitles look.\nYou can change this while subtitles are running.");
  expect(host.querySelector(".settings-help-control__button")?.getAttribute("aria-label")).toBe(I18N.settings.helpLabel);
  expect(host.querySelector(".settings-row__description, .settings-row__hint, [role=note], small, p")).toBeNull();
  expect(host.textContent).not.toContain("ⓘ");
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
});

it.each([
  ["Choose how subtitles look.", undefined],
  [undefined, "You can change this while subtitles are running."],
] as const)("retains a single optional explanation in a help tooltip", async (description, hint) => {
  await render(description, hint);
  expect(host.querySelectorAll(".settings-help-control")).toHaveLength(1);
  expect(host.querySelector(".settings-help-control__description")?.textContent).toBe(description ?? hint);
  expect(host.querySelector(".settings-row__description, .settings-row__hint")).toBeNull();
});

it("does not add an empty help icon to a clear control", async () => {
  await render();
  expect(host.querySelector(".settings-help-control")).toBeNull();
  expect(host.querySelector(".settings-row__label")?.textContent).toBe("Appearance");
  expect(host.querySelectorAll("button")).toHaveLength(1);
});

it("keeps ongoing feedback out of the label and control width", async () => {
  await act(() => root.render(<SettingsRow label="Output" feedback={<span role="status">No audio data</span>}><button>Choose</button></SettingsRow>));
  expect(host.querySelector('.settings-row > .settings-row__feedback [role="status"]')?.textContent).toBe("No audio data");
  expect(host.querySelector('.settings-row__control')?.textContent).toBe("Choose");
});

it("opens the combined help on keyboard focus and dismisses Escape while keeping the description available", async () => {
  await render("Choose how subtitles look.", "You can change this while subtitles are running.");
  const button = host.querySelector<HTMLButtonElement>(".settings-help-control__button")!;
  const descriptionId = button.getAttribute("aria-describedby")!;
  const matches = button.matches.bind(button);
  vi.spyOn(button, "matches").mockImplementation(selector => selector === ":focus-visible" || matches(selector));
  await act(() => button.focus());
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe("Choose how subtitles look.\nYou can change this while subtitles are running.");
  expect(host.contains(document.querySelector('[role="tooltip"]'))).toBe(false);
  await act(() => button.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  expect(document.activeElement).toBe(button);
  expect(document.getElementById(descriptionId)?.textContent).toBe("Choose how subtitles look.\nYou can change this while subtitles are running.");
});
