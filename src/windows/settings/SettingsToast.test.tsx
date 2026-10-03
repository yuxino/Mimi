// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { SettingsToastRegion } from "./SettingsToast";
import { useSettingsToast } from "./useSettingsToast";

type Notify = (message: string, failure?: boolean) => void;
let first: Notify, second: Notify;
let host: HTMLDivElement, root: Root | null;
function Action({ label, begin }: { label: string; begin: (notify: Notify) => void }) {
  const { beginToast } = useSettingsToast();
  return <button onClick={() => begin(beginToast())}>{label}</button>;
}
async function render(scopeKey = "diagnostics") {
  await act(() => root!.render(<>
    <Action label="Copy" begin={notify => { first = notify; }} />
    <Action label="Refresh" begin={notify => { second = notify; }} />
    <SettingsToastRegion scopeKey={scopeKey} />
  </>));
  // Flush jsdom's initial zero-delay task before counting notification timers.
  await act(() => vi.advanceTimersByTime(0));
}
async function click(index: number) { await act(() => host.querySelectorAll("button")[index].click()); }
beforeEach(() => {
  vi.useFakeTimers(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  if (root) await act(() => root!.unmount());
  root = null; host.remove(); setStoredUiLanguage("system"); vi.useRealTimers(); vi.unstubAllGlobals();
});

it("expires success after three seconds and errors after eight, with manual dismissal", async () => {
  await render(); await click(0); await act(() => first("Copied"));
  expect(host.querySelector('.settings-toast[role="status"]')?.textContent).toBe("Copied");
  await act(() => vi.advanceTimersByTime(3000));
  expect(host.querySelector(".settings-toast")).toBeNull();
  await click(1); await act(() => second("Unavailable", true));
  await act(() => vi.advanceTimersByTime(7999));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe("Unavailable");
  await act(() => host.querySelector<HTMLButtonElement>('[aria-label="Dismiss notification"]')!.click());
  expect(host.querySelector(".settings-toast")).toBeNull();
  expect(vi.getTimerCount()).toBe(0);
});

it("shows only the latest action and rejects an earlier action's late completion", async () => {
  await render(); await click(0); await click(1);
  await act(() => { second("Refreshed"); first("Old copy result"); });
  expect(host.querySelectorAll(".settings-toast")).toHaveLength(1);
  expect(host.querySelector(".settings-toast")?.textContent).toBe("Refreshed");
  expect(host.textContent).not.toContain("Old copy result");
});

it("clears on category navigation and prevents late results from reviving after blur", async () => {
  await render(); await click(0); await act(() => first("Copied"));
  await render("export");
  await act(() => first("Stale result"));
  expect(host.querySelector(".settings-toast")).toBeNull();
  await click(1); await act(() => second("Refreshed"));
  await act(() => window.dispatchEvent(new Event("blur")));
  await act(() => second("Late refresh"));
  expect(host.querySelector(".settings-toast")).toBeNull();
  expect(vi.getTimerCount()).toBe(0);
});

it("cleans timers on unmount and discards pending feedback from an unmounted action", async () => {
  await render(); await click(0); await act(() => first("Copied"));
  await act(() => root!.unmount()); root = null;
  await act(() => first("Late copy"));
  expect(host.querySelector(".settings-toast")).toBeNull();
  expect(vi.getTimerCount()).toBe(0);
});
