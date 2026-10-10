// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Select } from "../../components/Select";
import type { OverlayControlMode } from "../../lib/ipc";
import { OverlayControlWindow } from "./OverlayControlWindow";
import { SettingsToastRegion } from "../settings/SettingsToast";
import { useSettingsToast } from "../settings/useSettingsToast";
import { useStore } from "../../lib/store";

const native = vi.hoisted(() => ({
  listeners: new Set<(mode: OverlayControlMode) => void>(),
  hide: vi.fn(), toggle: vi.fn(), bounds: vi.fn(), choose: vi.fn(), failure: false,
}));
vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(),
  isTauri: true,
  overlayControlGetState: async () => "island",
  overlayControlSetIslandWidth: async () => {},
  overlayControlSetPopupBounds: native.bounds,
  overlayPopoverToggle: native.toggle,
  overlayPopoverHide: native.hide,
  listenOverlayControlMode: async (listener: (mode: OverlayControlMode) => void) => {
    native.listeners.add(listener);
    return () => native.listeners.delete(listener);
  },
}));
vi.mock("./OverlayControlPanel", () => ({
  OverlayControlPanel: () => {
    const { beginToast } = useSettingsToast();
    return <section className="overlay-control-panel">
      <SettingsToastRegion />
      <button data-testid="notify" onClick={() => beginToast()("Language adjusted", native.failure)}>Notify</button>
      <Select label="Language" value="en" options={[{ value: "en", label: "English" }, { value: "zh", label: "Chinese" }]} onChange={native.choose} />
    </section>;
  },
}));
let host: HTMLDivElement, root: Root;
let frames: Map<number, FrameRequestCallback>, frameId: number;
let wasLocked: boolean;
const mode = (value: OverlayControlMode) => native.listeners.forEach(listener => listener(value));
beforeEach(() => {
  wasLocked = useStore.getState().settings.isOverlayLocked;
  useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: true } }));
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  frames = new Map(); frameId = 0;
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { frames.set(++frameId, callback); return frameId; });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => { frames.delete(id); });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("settings-toast")
      ? { x: 260, y: 400, width: 360, height: 62, top: 400, left: 260, right: 620, bottom: 462 } as DOMRect
      : { x: 18, y: 16, width: 180, height: 30, top: 16, left: 18, right: 198, bottom: 46 } as DOMRect;
  });
  native.hide.mockReset().mockImplementation(async () => mode("island"));
  native.toggle.mockReset().mockImplementation(async () => mode("panel"));
  native.bounds.mockReset().mockResolvedValue(undefined); native.choose.mockReset();
  native.failure = false;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: wasLocked } }));
  host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});

it("embeds island/panel in one surface and keeps portaled picker selections inside the panel interaction", async () => {
  await act(async () => root.render(<OverlayControlWindow embedded />));
  const surface = host.querySelector(".overlay-control-embedded")!;
  expect(surface.getAttribute("data-mode")).toBe("island");
  await act(async () => host.querySelector<HTMLButtonElement>(".overlay-control-island")!.click());
  expect(surface.getAttribute("data-mode")).toBe("panel");
  await act(async () => host.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
  const menu = document.querySelector(".mimi-select__menu")!;
  expect(surface.contains(menu)).toBe(false);
  await act(async () => {
    const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(0));
  });
  expect(native.bounds).toHaveBeenLastCalledWith({ x: 18, y: 16, width: 180, height: 30 }, null);
  const option = menu.querySelectorAll<HTMLElement>('[role="option"]')[1];
  await act(async () => {
    option.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    option.click();
  });
  expect(native.choose).toHaveBeenCalledWith("zh");
  expect(native.hide).not.toHaveBeenCalled();
  await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
  expect(native.hide).toHaveBeenCalledOnce();
  expect(surface.getAttribute("data-mode")).toBe("island");
  expect(document.querySelector(".mimi-select__menu")).toBeNull();
  await act(async () => mode("hidden"));
  expect(surface.getAttribute("data-mode")).toBe("hidden");
  expect(host.querySelector(".overlay-control-panel")).toBeNull();
  expect(native.bounds).toHaveBeenLastCalledWith(null);
});

it("does not install embedded dismissal or input-region reporting for a separate control window", async () => {
  await act(async () => root.render(<OverlayControlWindow />));
  await act(async () => mode("panel"));
  await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
  expect(native.hide).not.toHaveBeenCalled();
  expect(native.bounds).not.toHaveBeenCalled();
  expect(host.querySelector(".overlay-control-embedded")).toBeNull();
});

it.each([false, true])("keeps a locked notification dismissible alongside a picker (failure=%s)", async failure => {
  native.failure = failure;
  await act(async () => root.render(<OverlayControlWindow embedded />));
  await act(async () => mode("panel"));
  await act(async () => host.querySelector<HTMLButtonElement>('[data-testid="notify"]')!.click());
  const notice = host.querySelector<HTMLElement>(".settings-toast")!;
  expect(notice.getAttribute("role")).toBe(failure ? "alert" : "status");
  await act(async () => host.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
  await act(async () => {
    const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(0));
  });
  expect(native.bounds).toHaveBeenLastCalledWith(
    { x: 18, y: 16, width: 180, height: 30 },
    { x: 260, y: 400, width: 360, height: 62 },
  );
  const dismiss = notice.querySelector<HTMLButtonElement>("button")!;
  await act(async () => {
    dismiss.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    dismiss.click();
  });
  expect(host.querySelector(".settings-toast")).toBeNull();
  expect(native.hide).not.toHaveBeenCalled();
  expect(host.querySelector(".overlay-control-embedded")?.getAttribute("data-mode")).toBe("panel");
  await act(async () => {
    const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(0));
  });
  expect(native.bounds).toHaveBeenLastCalledWith(null, null);
  await act(async () => host.querySelector<HTMLButtonElement>('[data-testid="notify"]')!.click());
  await act(async () => mode("hidden"));
  expect(host.querySelector(".settings-toast")).toBeNull();
  expect(native.bounds).toHaveBeenLastCalledWith(null);
});
