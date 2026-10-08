// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { LocalModelLibrary } from "./LocalModelLibrary";
import { setStoredUiLanguage } from "../../lib/i18n";
import { localModelsStatus, downloadLocalModel, cancelLocalModelDownload, deleteLocalModel } from "../../lib/ipc";
import type { LocalModelStatus } from "../../lib/types";
vi.mock("../../lib/ipc", () => ({ setOverlayPointerCursor: vi.fn(), localModelsStatus: vi.fn(), downloadLocalModel: vi.fn(), cancelLocalModelDownload: vi.fn(), deleteLocalModel: vi.fn() }));
let host: HTMLDivElement, root: Root;
const base: LocalModelStatus = { id: "qwenSmall", name: "Qwen3-ASR 0.6B", phase: "missing", installed: false, inUse: false, downloadBytes: 712778752, downloadedBytes: 0, error: null };
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); setStoredUiLanguage("en");
  vi.mocked(localModelsStatus).mockReset().mockResolvedValue({ available: true, models: [{ ...base }] });
  vi.mocked(downloadLocalModel).mockReset().mockResolvedValue(); vi.mocked(cancelLocalModelDownload).mockReset().mockResolvedValue(); vi.mocked(deleteLocalModel).mockReset().mockResolvedValue();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
const button = (text: string) => [...host.querySelectorAll("button")].find(button => button.textContent?.includes(text))!;
it("downloads only after a click and exposes cancellation with progress", async () => {
  await act(async () => root.render(<LocalModelLibrary />));
  expect(downloadLocalModel).not.toHaveBeenCalled(); expect(host.textContent).toContain("713 MB");
  vi.mocked(localModelsStatus).mockResolvedValue({ available: true, models: [{ ...base, phase: "downloading", downloadedBytes: base.downloadBytes / 2 }] });
  await act(async () => button("Download").click());
  expect(downloadLocalModel).toHaveBeenCalledExactlyOnceWith("qwenSmall"); expect(host.textContent).toContain("50%");
  await act(async () => button("Cancel").click()); expect(cancelLocalModelDownload).toHaveBeenCalledExactlyOnceWith("qwenSmall");
});
it("uses installed models and confirms deletion without removing profiles", async () => {
  vi.mocked(localModelsStatus).mockResolvedValue({ available: true, models: [{ ...base, phase: "installed", installed: true }] });
  const use = vi.fn().mockResolvedValue(undefined);
  await act(async () => root.render(<LocalModelLibrary onUse={use} />));
  await act(async () => button("Use model").click()); expect(use).toHaveBeenCalledWith("qwenSmall", base.name);
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label^="Delete model"]')!.click());
  expect(deleteLocalModel).not.toHaveBeenCalled();
  expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("configurations remain");
  const confirm = [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')].find(button => button.textContent === "Delete model")!;
  await act(async () => confirm.click()); expect(deleteLocalModel).toHaveBeenCalledExactlyOnceWith("qwenSmall");
});
it("prevents deleting a leased model", async () => {
  vi.mocked(localModelsStatus).mockResolvedValue({ available: true, models: [{ ...base, phase: "installed", installed: true, inUse: true }] });
  await act(async () => root.render(<LocalModelLibrary />));
  expect(host.querySelector<HTMLButtonElement>('[aria-label^="Delete model"]')!.disabled).toBe(true);
});
it("offers retry after a failed download", async () => {
  vi.mocked(localModelsStatus).mockResolvedValue({ available: true, models: [{ ...base, phase: "error", error: "local_model_integrity_failed" }] });
  await act(async () => root.render(<LocalModelLibrary />)); expect(host.textContent).toContain("Verification failed");
  await act(async () => button("Retry").click()); expect(downloadLocalModel).toHaveBeenCalledExactlyOnceWith("qwenSmall");
});

it("guides unsupported devices to user-owned models without unusable downloads", async () => {
  vi.mocked(localModelsStatus).mockResolvedValue({ available: false, models: [{ ...base }] });
  const own = vi.fn();
  await act(async () => root.render(<LocalModelLibrary onUseOwn={own} />));
  expect(host.textContent).toContain("Built-in Qwen models require");
  expect(host.querySelector(".local-models__privacy")?.textContent).toContain("this computer");
  expect(host.querySelector(".local-models__row")).toBeNull();
  expect(host.querySelector('[aria-label^="Download"]')).toBeNull();
  await act(async () => button("Use your own model").click());
  expect(own).toHaveBeenCalledOnce();
  expect(downloadLocalModel).not.toHaveBeenCalled();
});
it("retains deletion of installed models on unsupported devices", async () => {
  vi.mocked(localModelsStatus).mockResolvedValue({ available: false, models: [{ ...base, phase: "installed", installed: true }] });
  await act(async () => root.render(<LocalModelLibrary onUse={vi.fn()} />));
  expect(host.querySelector<HTMLButtonElement>('[aria-label^="Use model"]')!.disabled).toBe(true);
  expect(host.querySelector<HTMLButtonElement>('[aria-label^="Delete model"]')!.disabled).toBe(false);
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label^="Delete model"]')!.click());
  expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
  expect(deleteLocalModel).not.toHaveBeenCalled();
});
it("keeps the own-model action disabled while configuration changes are blocked", async () => {
  vi.mocked(localModelsStatus).mockResolvedValue({ available: false, models: [{ ...base }] });
  const own = vi.fn();
  await act(async () => root.render(<LocalModelLibrary disabled onUseOwn={own} />));
  await act(async () => button("Use your own model").click());
  expect(own).not.toHaveBeenCalled();
});
