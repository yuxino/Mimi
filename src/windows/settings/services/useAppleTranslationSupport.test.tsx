// @vitest-environment jsdom
import { act, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { getAppleTranslationStatus, getAppleTranslationSupport } from "../../../lib/ipc";
import type { AppleTranslationStatus, AppleTranslationSupport, SourceLanguage } from "../../../lib/types";
import { useAppleTranslationStatus, useAppleTranslationSupport } from "./useAppleTranslationSupport";

vi.mock("../../../lib/ipc", async original => ({ ...await original<typeof import("../../../lib/ipc")>(), isTauri: true, getAppleTranslationStatus: vi.fn(), getAppleTranslationSupport: vi.fn() }));
const available: AppleTranslationSupport = { available: true, sourceLanguages: ["en", "ja"], targetLanguages: ["en", "ja"] };
let host: HTMLDivElement, root: Root;
let current: ReturnType<typeof useAppleTranslationSupport>, pair: ReturnType<typeof useAppleTranslationStatus>;
function Harness({ visible, source = "en", profileId = "profile" }: { visible: boolean; source?: SourceLanguage; profileId?: string }) {
  const support = useAppleTranslationSupport(visible);
  const status = useAppleTranslationStatus(profileId, source, "ja", visible);
  useEffect(() => { current = support; pair = status; }, [support, status]);
  return null;
}
const render = async (visible = true, source: SourceLanguage = "en", profileId = "profile") => { await act(async () => root.render(<Harness visible={visible} source={source} profileId={profileId} />)); };
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (reason: Error) => void; const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; }); return { promise, resolve, reject }; }
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(getAppleTranslationSupport).mockReset().mockResolvedValue(available);
  vi.mocked(getAppleTranslationStatus).mockReset().mockResolvedValue("installed");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("does not query hidden views and does not revive cached readiness on reopening", async () => {
  await render(false);
  expect(getAppleTranslationSupport).not.toHaveBeenCalled();
  expect(getAppleTranslationStatus).not.toHaveBeenCalled();
  await render();
  expect(current.support).toEqual(available);
  expect(pair.status).toBe("installed");
  await render(false);
  const support = deferred<AppleTranslationSupport>(), status = deferred<AppleTranslationStatus>();
  vi.mocked(getAppleTranslationSupport).mockReturnValueOnce(support.promise);
  vi.mocked(getAppleTranslationStatus).mockReturnValueOnce(status.promise);
  await render();
  expect(current.loading).toBe(true);
  expect(pair.loading).toBe(true);
  expect(pair.status).toBeNull();
  await act(async () => { support.resolve(available); status.resolve("supported"); });
  expect(pair.status).toBe("supported");
});

it("discards a late result after profile or source changes", async () => {
  const old = deferred<AppleTranslationStatus>(), latest = deferred<AppleTranslationStatus>();
  vi.mocked(getAppleTranslationStatus).mockReturnValueOnce(old.promise).mockReturnValueOnce(latest.promise);
  await render();
  await render(true, "fr", "other");
  await act(async () => old.resolve("installed"));
  expect(pair.status).toBeNull();
  await act(async () => latest.resolve("unsupported"));
  expect(pair.status).toBe("unsupported");
});

it("keeps explicit preparation status when an older refresh finishes later", async () => {
  await render();
  const old = deferred<AppleTranslationStatus>();
  vi.mocked(getAppleTranslationStatus).mockReturnValueOnce(old.promise);
  let refresh!: Promise<void>;
  await act(async () => { refresh = pair.refresh(); });
  await act(async () => pair.update("installed"));
  await act(async () => { old.resolve("supported"); await refresh; });
  expect(pair.status).toBe("installed");
});

it("sanitizes failed support and pair queries and recovers on explicit retry", async () => {
  vi.mocked(getAppleTranslationSupport).mockRejectedValueOnce(new Error("private-support"));
  vi.mocked(getAppleTranslationStatus).mockRejectedValueOnce(new Error("private-status"));
  await render();
  expect(current.failed).toBe(true);
  expect(current.support).toBeNull();
  expect(pair.failed).toBe(true);
  await act(async () => { await current.refresh(); await pair.refresh(); });
  expect(current.support).toEqual(available);
  expect(pair.status).toBe("installed");
});

it("shows pending capability and pair retries and shares repeated requests in the same context", async () => {
  vi.mocked(getAppleTranslationSupport).mockRejectedValueOnce(new Error("private-support"));
  vi.mocked(getAppleTranslationStatus).mockRejectedValueOnce(new Error("private-status"));
  await render();
  const support = deferred<AppleTranslationSupport>(), status = deferred<AppleTranslationStatus>();
  vi.mocked(getAppleTranslationSupport).mockReturnValueOnce(support.promise);
  vi.mocked(getAppleTranslationStatus).mockReturnValueOnce(status.promise);
  let retries!: Promise<void>[];
  await act(async () => { retries = [current.refresh(), current.refresh(), pair.refresh(), pair.refresh()]; });
  expect(getAppleTranslationSupport).toHaveBeenCalledTimes(2);
  expect(getAppleTranslationStatus).toHaveBeenCalledTimes(2);
  expect(current.loading).toBe(true);
  expect(current.failed).toBe(false);
  expect(pair.loading).toBe(true);
  expect(pair.failed).toBe(false);
  expect(pair.status).toBeNull();
  await act(async () => { support.resolve(available); status.resolve("supported"); await Promise.all(retries); });
  expect(current.support).toEqual(available);
  expect(current.loading).toBe(false);
  expect(pair.status).toBe("supported");
  expect(pair.loading).toBe(false);
});

it("does not send explicit refreshes while hidden", async () => {
  await render(false);
  await act(async () => { await current.refresh(); await pair.refresh(); });
  expect(getAppleTranslationSupport).not.toHaveBeenCalled();
  expect(getAppleTranslationStatus).not.toHaveBeenCalled();
});

it.each(["resolve", "reject"] as const)("keeps a fresh reopened query pending after an older retry %s", async outcome => {
  await render();
  const oldSupport = deferred<AppleTranslationSupport>(), oldStatus = deferred<AppleTranslationStatus>();
  vi.mocked(getAppleTranslationSupport).mockReturnValueOnce(oldSupport.promise);
  vi.mocked(getAppleTranslationStatus).mockReturnValueOnce(oldStatus.promise);
  let old!: Promise<void>[];
  await act(async () => { old = [current.refresh(), pair.refresh()]; });
  await render(false);
  const latestSupport = deferred<AppleTranslationSupport>(), latestStatus = deferred<AppleTranslationStatus>();
  vi.mocked(getAppleTranslationSupport).mockReturnValueOnce(latestSupport.promise);
  vi.mocked(getAppleTranslationStatus).mockReturnValueOnce(latestStatus.promise);
  await render();
  await act(async () => {
    if (outcome === "resolve") { oldSupport.resolve(available); oldStatus.resolve("installed"); }
    else { oldSupport.reject(new Error("private-old-support")); oldStatus.reject(new Error("private-old-status")); }
    await Promise.all(old);
  });
  expect(current.loading).toBe(true);
  expect(current.support).toBeNull();
  expect(pair.loading).toBe(true);
  expect(pair.status).toBeNull();
  await act(async () => { latestSupport.resolve(available); latestStatus.resolve("supported"); });
  expect(current.support).toEqual(available);
  expect(pair.status).toBe("supported");
});

it("starts a fresh status retry after an explicit preparation supersedes the old query", async () => {
  await render();
  const old = deferred<AppleTranslationStatus>(), latest = deferred<AppleTranslationStatus>();
  vi.mocked(getAppleTranslationStatus).mockReturnValueOnce(old.promise).mockReturnValueOnce(latest.promise);
  let oldRetry!: Promise<void>, latestRetry!: Promise<void>;
  await act(async () => { oldRetry = pair.refresh(); });
  await act(async () => pair.update("installed"));
  await act(async () => { latestRetry = pair.refresh(); });
  expect(getAppleTranslationStatus).toHaveBeenCalledTimes(3);
  expect(pair.loading).toBe(true);
  await act(async () => { old.resolve("unsupported"); await oldRetry; });
  expect(pair.loading).toBe(true);
  expect(pair.status).toBeNull();
  await act(async () => { latest.resolve("installed"); await latestRetry; });
  expect(pair.status).toBe("installed");
});
