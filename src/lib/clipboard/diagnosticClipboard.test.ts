import { afterEach, expect, it, vi } from "vitest";
import { writeDiagnosticClipboard } from "./diagnosticClipboard";

afterEach(() => vi.unstubAllGlobals());

it("starts the clipboard write before async IPC resolves, preserving the click gesture", async () => {
  let resolve!: (value: string) => void;
  const report = new Promise<string>((done) => { resolve = done; });
  let data!: Record<string, Promise<Blob>>;
  vi.stubGlobal("ClipboardItem", class { constructor(value: Record<string, Promise<Blob>>) { data = value; } });
  const write = vi.fn(async () => { await data["text/plain"]; });
  vi.stubGlobal("navigator", { clipboard: { write } });
  const result = writeDiagnosticClipboard(report);
  expect(write).toHaveBeenCalledOnce();
  resolve("safe snapshot");
  await result;
  expect(await (await data["text/plain"]).text()).toBe("safe snapshot");
});

it("propagates denied clipboard permission without reporting success", async () => {
  vi.stubGlobal("ClipboardItem", undefined);
  vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) } });
  await expect(writeDiagnosticClipboard(Promise.resolve("safe snapshot"))).rejects.toThrow("denied");
});

it("does not copy when preparation fails", async () => {
  vi.stubGlobal("ClipboardItem", undefined);
  const writeText = vi.fn();
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  await expect(writeDiagnosticClipboard(Promise.reject(new Error("not available")))).rejects.toThrow();
  expect(writeText).not.toHaveBeenCalled();
});
