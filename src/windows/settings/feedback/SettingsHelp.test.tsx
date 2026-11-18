// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SettingsHelp } from "./SettingsHelp";

let host: HTMLDivElement;
let root: Root;
const explanation = "Credentials are stored in the operating system's secure storage.";

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function render(id?: string) {
  await act(() => root.render(<SettingsHelp text={explanation} label="More information" id={id} />));
}

function trigger() { return host.querySelector<HTMLButtonElement>("button")!; }
function popup() { return document.querySelector<HTMLElement>('[role="tooltip"]'); }

it("keeps the accessible description available to adjacent inputs before the popup opens", async () => {
  await render("credential-storage-help");
  expect(trigger().getAttribute("aria-label")).toBe("More information");
  expect(trigger().getAttribute("aria-describedby")).toBe("credential-storage-help");
  expect(document.getElementById("credential-storage-help")?.textContent).toBe(explanation);
  expect(popup()).toBeNull();
  expect(trigger().querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
});

it("shows long help in a portal on hover and removes the popup on pointer leave", async () => {
  await render();
  await act(() => trigger().dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
  expect(popup()?.textContent).toBe(explanation);
  expect(popup()?.classList.contains("settings-help-tooltip")).toBe(true);
  expect(host.contains(popup())).toBe(false);
  const descriptionId = trigger().getAttribute("aria-describedby")!;
  await act(() => trigger().dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
  expect(popup()).toBeNull();
  expect(document.getElementById(descriptionId)?.textContent).toBe(explanation);
});

it("opens on keyboard focus and dismisses with Escape without moving focus or removing the description", async () => {
  await render();
  const button = trigger();
  const matches = button.matches.bind(button);
  vi.spyOn(button, "matches").mockImplementation(selector => selector === ":focus-visible" || matches(selector));
  await act(() => button.focus());
  expect(popup()?.textContent).toBe(explanation);
  await act(() => button.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(popup()).toBeNull();
  expect(document.activeElement).toBe(button);
  expect(document.getElementById(button.getAttribute("aria-describedby")!)?.textContent).toBe(explanation);
});

it("preserves generated description ids across rerenders and updates the description text", async () => {
  await render();
  const descriptionId = trigger().getAttribute("aria-describedby")!;
  await act(() => root.render(<SettingsHelp text="Updated requirements" label="More information" icon="shield-check" />));
  expect(trigger().getAttribute("aria-describedby")).toBe(descriptionId);
  expect(document.getElementById(descriptionId)?.textContent).toBe("Updated requirements");
});
