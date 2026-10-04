// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { OverlayTranslationService } from "./OverlayTranslationService";
import { translationService } from "./translationService";
import type { ServiceProfile, TargetLanguage } from "../../lib/types";

let root: Root, host: HTMLDivElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  setStoredUiLanguage("en");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove(); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});

async function render(overrides: Partial<ServiceProfile> = {}, targetLanguage: TargetLanguage = "zh", disabled = false) {
  const service = translationService({
    profiles: [{ id: "test", name: "Configuration name", provider: "alibabaCloud", credentialState: "present", ...overrides }],
    activeProfileId: "test", targetLanguage,
  })!;
  const click = vi.fn();
  await act(async () => root.render(<OverlayTranslationService service={service} onClick={click} disabled={disabled} />));
  return click;
}

it("shows both services and keeps full role labels accessible while the visible names truncate", async () => {
  const name = "B 站 / long custom translator 日本語 🌸 ".repeat(2);
  const click = await render({ textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: name } });
  const button = host.querySelector("button")!;
  expect([...host.querySelectorAll(".overlay-service__name")].map(node => node.textContent))
    .toEqual(["Alibaba Cloud", name.trim()]);
  expect([...host.querySelectorAll("[data-provider]")].map(node => node.getAttribute("data-provider")))
    .toEqual(["alibabaCloud", "openAICompatible"]);
  expect(button.getAttribute("aria-label")).toContain("Speech recognition: Alibaba Cloud");
  expect(button.getAttribute("aria-label")).toContain(`Text translation: ${name.trim()}`);
  await act(async () => { button.focus(); button.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })); });
  expect(document.querySelector('[role="tooltip"]')?.textContent).toContain(name.trim());
  await act(async () => button.click());
  expect(click).toHaveBeenCalledOnce();
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
});

it("combines a built-in service and updates both provider and route after repeat selection", async () => {
  await render({ textTranslation: "followService" });
  expect(host.querySelectorAll(".overlay-service__stage")).toHaveLength(1);
  expect(host.querySelector('[data-stage="combined"]')?.textContent).toBe("Alibaba Cloud");
  await render({ provider: "customOpenAIASR", textTranslation: "deepL" });
  expect([...host.querySelectorAll("[data-provider]")].map(node => node.getAttribute("data-provider")))
    .toEqual(["customOpenAIASR", "deepL"]);
  await render({ provider: "openAIRealtime", textTranslation: "followService" });
  expect(host.querySelectorAll(".overlay-service__stage")).toHaveLength(1);
  expect(host.querySelector('[data-stage="combined"] [data-provider="openAIRealtime"]')).not.toBeNull();
});

it("shows only recognition for original mode and disables settings while an action is pending", async () => {
  const click = await render({ textTranslation: "chatMock", textTranslationNames: { chatMock: "Unused translation name" } }, "original", true);
  expect(host.querySelectorAll(".overlay-service__stage")).toHaveLength(1);
  expect(host.querySelector('[data-stage="recognition"]')?.textContent).toBe("Alibaba Cloud");
  expect(host.querySelector("button")?.getAttribute("aria-label")).toContain("Text translation: Original only");
  expect(host.innerHTML).not.toContain("Unused translation name");
  await act(async () => host.querySelector("button")!.click());
  expect(click).not.toHaveBeenCalled();
});
