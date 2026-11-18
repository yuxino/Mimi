// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage, UI_LANGUAGES } from "../../../lib/i18n";
import { openAlibabaSetupPage } from "../../../lib/ipc";
import { AlibabaRealtimeSettings } from "./AlibabaRealtimeSettings";
import { AlibabaSetupLink } from "./AlibabaSetupLink";
import { SettingsToastRegion } from "../SettingsToast";

vi.mock("../../../lib/ipc", () => ({ isTauri: false, openAlibabaSetupPage: vi.fn(), setOverlayPointerCursor: vi.fn() }));
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(openAlibabaSetupPage).mockReset().mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  setStoredUiLanguage("en"); vi.unstubAllGlobals();
});

it.each(UI_LANGUAGES)("opens only explicit Alibaba destinations with compact controls in %s", async language => {
  setStoredUiLanguage(language);
  await act(async () => root.render(<><AlibabaSetupLink page="apiKey" /><AlibabaSetupLink page="workspace" /><AlibabaSetupLink page="guide" /></>));
  const buttons = host.querySelectorAll<HTMLButtonElement>("button");
  expect(buttons).toHaveLength(3);
  expect(host.querySelector("p")).toBeNull();
  expect(openAlibabaSetupPage).not.toHaveBeenCalled();
  for (const [index, destination] of ["apiKey", "workspace", "guide"].entries()) {
    expect(buttons[index].textContent?.trim()).toBeTruthy();
    await act(async () => buttons[index].click());
    expect(openAlibabaSetupPage).toHaveBeenLastCalledWith(destination);
  }
});

it("reports an opening failure through the shared sanitized toast", async () => {
  setStoredUiLanguage("en");
  vi.mocked(openAlibabaSetupPage).mockRejectedValue(new Error("private-native-detail"));
  await act(async () => root.render(<><AlibabaSetupLink page="workspace" /><SettingsToastRegion /></>));
  await act(async () => host.querySelector<HTMLButtonElement>(".alibaba-setup-link")!.click());
  expect(host.textContent).toContain("Could not open the Model Studio setup page");
  expect(host.textContent).not.toContain("private-native-detail");
});

it("keeps a disabled setup control inactive", async () => {
  await act(async () => root.render(<AlibabaSetupLink page="apiKey" disabled />));
  const button = host.querySelector<HTMLButtonElement>("button")!;
  expect(button.disabled).toBe(true);
  await act(async () => button.click());
  expect(openAlibabaSetupPage).not.toHaveBeenCalled();
});


it.each(UI_LANGUAGES)("labels both translation modes in the interface language in %s", async language => {
  setStoredUiLanguage(language);
  await act(async () => root.render(<AlibabaRealtimeSettings
    profile={{ id: "synthetic", name: "Alibaba", provider: "alibabaCloud", credentialState: "present", alibabaRealtimeMode: "liveTranslate", alibabaWorkspaceId: "llm-test-workspace" }}
    disabled={false} onSave={vi.fn()} />));
  const selector = host.querySelector<HTMLButtonElement>('[role="combobox"]')!;
  expect(selector.textContent?.trim()).toBeTruthy();
  expect(selector.textContent).not.toMatch(/LiveTranslate|ASR \+ MT/);
  await act(async () => selector.click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options).toHaveLength(2);
  expect(options[0].querySelector(".mimi-select__label")?.textContent).toBe(selector.textContent);
  expect(options[0].getAttribute("aria-selected")).toBe("true");
  expect(options[1].querySelector(".mimi-select__label")?.textContent?.trim()).toBeTruthy();
  const descriptions = options.map(option => option.querySelector(".mimi-select__description")?.textContent);
  expect(descriptions.every(description => typeof description === "string" && description.trim().length > 0)).toBe(true);
  if (language === "zh") {
    expect(descriptions).toEqual([
      "边听边译，同时输出原文和译文。",
      "可更换翻译服务",
    ]);
  }
  expect(options.map(option => option.textContent).join(" ")).not.toContain("Omni");
  expect(openAlibabaSetupPage).not.toHaveBeenCalled();
});


it.each([undefined, "disabled"] as const)("lists LiveTranslate first while preserving ASR + MT with stored mode %s", async mode => {
  setStoredUiLanguage("en");
  const onSave = vi.fn();
  await act(async () => root.render(<AlibabaRealtimeSettings
    profile={{ id: "synthetic", name: "Alibaba", provider: "alibabaCloud", credentialState: "present", alibabaRealtimeMode: mode }}
    disabled={false} onSave={onSave} />));
  const selector = host.querySelector<HTMLButtonElement>('[role="combobox"]')!;
  expect(selector.textContent).toBe("ASR + MT");
  await act(async () => selector.click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options[0].getAttribute("aria-selected")).toBe("false");
  expect(options[1].getAttribute("aria-selected")).toBe("true");
  expect(onSave).not.toHaveBeenCalled();
});
