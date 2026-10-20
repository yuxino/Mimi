// @vitest-environment jsdom
import { act, StrictMode, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N } from "../../lib/i18n";
import { AutoSaveNameField } from "./AutoSaveNameField";
import { NameDraftContext, NameDraftOwner, type NameDraftKey } from "./nameDraftOwner";

let host: HTMLDivElement, root: Root;
const save = vi.fn<(name: string) => Promise<unknown>>();
const submit = vi.fn();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  save.mockReset().mockResolvedValue({}); submit.mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(() => root.unmount()); host.remove();
  vi.useRealTimers(); vi.unstubAllGlobals();
});
async function render(props: Partial<ComponentProps<typeof AutoSaveNameField>> = {}, key = "first") {
  await act(() => root.render(<form onSubmit={event => { event.preventDefault(); submit(); }}>
    <AutoSaveNameField key={key} id="name" label="Name" value="Original" placeholder="Default" onSave={save} {...props} />
    <button type="submit">Credentials</button>
  </form>));
}
const input = () => host.querySelector<HTMLInputElement | HTMLTextAreaElement>("#name")!;
async function change(value: string) {
  await act(() => {
    input().focus();
    const prototype = input() instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function tick() { await act(async () => vi.advanceTimersByTimeAsync(350)); }
async function key(name: string, extra: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true, ...extra });
  await act(async () => input().dispatchEvent(event));
  return event;
}
function deferred() {
  let resolve!: (value: unknown) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<unknown>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function renderOwned(owner: NameDraftOwner, profileId = "first", props: Partial<ComponentProps<typeof AutoSaveNameField>> = {}, shown = true) {
  await act(() => root.render(<StrictMode><NameDraftContext.Provider value={{ owner, profileId }}>
    {shown ? <AutoSaveNameField key={profileId} id="name" draftKey="profile" label="Name" value="Original" placeholder="Default" onSave={save} {...props} /> : <div>List</div>}
  </NameDraftContext.Provider></StrictMode>));
}

it.each(["profile", "recognition", "translation:deepL"] as NameDraftKey[])("reconciles a newer authoritative %s name after an older save reply and keeps it across navigation", async draftKey => {
  const owner = new NameDraftOwner(), pending = deferred();
  save.mockReturnValueOnce(pending.promise);
  await renderOwned(owner, "first", { draftKey });
  await change("Saved A"); await tick();
  const field = input();
  await renderOwned(owner, "first", { draftKey, value: "Saved A" });
  await renderOwned(owner, "first", { draftKey, value: "Newer B" });
  expect(input().value).toBe("Saved A");
  await act(async () => pending.resolve({}));
  expect(input()).toBe(field);
  expect(input().value).toBe("Newer B");
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await renderOwned(owner, "first", { draftKey, value: "Newer B" }, false);
  await renderOwned(owner, "first", { draftKey, value: "Newer B" });
  expect(input().value).toBe("Newer B");
  await tick();
  expect(save).toHaveBeenCalledExactlyOnceWith("Saved A");
});

it("distinguishes a new authoritative return to the initial name from having no echo", async () => {
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await render(); await change("Saved A"); await tick();
  await render({ value: "Saved A" }); await render({ value: "Original" });
  await act(async () => pending.resolve({}));
  expect(input().value).toBe("Original");
  await tick(); expect(save).toHaveBeenCalledExactlyOnceWith("Saved A");
});

it.each(["recognition", "translation:deepL"] as NameDraftKey[])("accepts an empty authoritative %s alias as a real update", async draftKey => {
  const owner = new NameDraftOwner(), pending = deferred();
  save.mockReturnValueOnce(pending.promise);
  await renderOwned(owner, "first", { draftKey });
  await change("Saved A"); await tick();
  await renderOwned(owner, "first", { draftKey, value: "Saved A" });
  await renderOwned(owner, "first", { draftKey, value: "" });
  await act(async () => pending.resolve({}));
  expect(input().value).toBe("");
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await tick(); expect(save).toHaveBeenCalledExactlyOnceWith("Saved A");
});

it("accepts a new authoritative name during a pending save even without that save's prop echo", async () => {
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await render(); await change("Saved A"); await tick();
  await render({ value: "Newer B" });
  await act(async () => pending.resolve({}));
  expect(input().value).toBe("Newer B");
  await tick(); expect(save).toHaveBeenCalledExactlyOnceWith("Saved A");
});

it("keeps a successful name when no source echo arrives", async () => {
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await render(); await change("Saved A"); await tick();
  await act(async () => pending.resolve({}));
  await render({ value: "Original" });
  expect(input().value).toBe("Saved A");
  expect(save).toHaveBeenCalledExactlyOnceWith("Saved A");
});

it.each(["current", "older"] as const)("distinguishes %s own acknowledgement after an external pending-save update", async kind => {
  const owner = new NameDraftOwner();
  await renderOwned(owner); await change("Older own name"); await tick();
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await change("Current own name"); await tick();
  await renderOwned(owner, "first", { value: "External name" });
  await renderOwned(owner, "first", { value: kind === "current" ? "Current own name" : "Older own name" });
  await act(async () => pending.resolve({}));
  expect(input().value).toBe(kind === "current" ? "Current own name" : "External name");
  await tick(); expect(save).toHaveBeenCalledTimes(2);
});

it.each(["New typing", "Saved A", "  Newer B  "])("preserves a later local revision %j over pending-save source updates", async latest => {
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await render(); await change("Saved A"); await tick();
  await render({ value: "Saved A" }); await render({ value: "Newer B" });
  await change("Intermediate local edit"); await change(latest);
  await tick(); // Queue the newer edit while the first write is still pending.
  const field = input();
  await act(() => field.setSelectionRange(2, 2));
  await act(async () => pending.resolve({}));
  expect(input()).toBe(field); expect(input().value).toBe(latest);
  expect(input().selectionStart).toBe(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(save).toHaveBeenCalledTimes(latest.trim() === "Newer B" ? 1 : 2);
  if (latest.trim() !== "Newer B") expect(save).toHaveBeenLastCalledWith(latest.trim());
});

it("preserves active composition when an earlier save succeeds after an external update", async () => {
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await render(); await change("Saved A"); await tick();
  await render({ value: "Saved A" }); await render({ value: "Newer B" });
  await act(() => input().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
  await act(async () => pending.resolve({}));
  expect(input().value).toBe("Saved A");
  await tick(); expect(save).toHaveBeenCalledExactlyOnceWith("Saved A");
});

it("preserves raw whitespace for an authoritative name equal to the owned trimmed draft", async () => {
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await render(); await change("  Saved A  "); await tick();
  await render({ value: "Saved A" }); // Consume the current own acknowledgement.
  await render({ value: "External interim" }); await render({ value: "Saved A" });
  await act(() => input().setSelectionRange(3, 3));
  await act(async () => pending.resolve({}));
  expect(input().value).toBe("  Saved A  "); expect(input().selectionStart).toBe(3);
});

it("preserves interrupted composition across a late save and waits for explicit confirmation", async () => {
  const owner = new NameDraftOwner(), pending = deferred();
  save.mockReturnValueOnce(pending.promise);
  await renderOwned(owner); await change("Saved A"); await tick();
  await renderOwned(owner, "first", { value: "Saved A" });
  await renderOwned(owner, "first", { value: "Newer B" });
  await act(() => input().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
  await renderOwned(owner, "first", { value: "Newer B" }, false);
  await act(async () => pending.resolve({}));
  await renderOwned(owner, "first", { value: "Newer B" });
  expect(input().value).toBe("Saved A");
  await tick(); expect(save).toHaveBeenCalledExactlyOnceWith("Saved A");
  await key("Enter");
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith("Saved A");
});

it.each(["Original", "  Newer B  "])("keeps pre-save edits %j against a newer authoritative baseline", async latest => {
  await render(); await change("Unsaved A");
  await render({ value: "Newer B" });
  expect(input().value).toBe("Unsaved A");
  await change(latest); await tick();
  expect(input().value).toBe(latest);
  expect(save).toHaveBeenCalledTimes(latest.trim() === "Newer B" ? 0 : 1);
  if (latest.trim() !== "Newer B") expect(save).toHaveBeenLastCalledWith(latest);
});

it("keeps a failed dirty draft and retry while accepting an idle external baseline", async () => {
  save.mockRejectedValueOnce(new Error("private-write-failure"));
  await render(); await change("Failed A"); await tick();
  const error = host.querySelector('[role="alert"]');
  await render({ value: "Newer B" });
  expect(input().value).toBe("Failed A");
  expect(host.querySelector('[role="alert"]')).toBe(error);
  await act(async () => host.querySelector<HTMLButtonElement>('.auto-save-name-field__error button')!.click());
  expect(save).toHaveBeenLastCalledWith("Failed A");
  expect(save).toHaveBeenCalledTimes(2);
});

it("does not let an older own acknowledgement replace a dirty external baseline after blur", async () => {
  await render(); await change("Older own name"); await tick();
  save.mockRejectedValueOnce(new Error("private-write-failure"));
  await change("Failed A"); await tick();
  await render({ value: "Newer B" });
  await act(() => host.querySelector<HTMLButtonElement>('button[type="submit"]')!.focus());
  await render({ value: "Older own name" });
  expect(input().value).toBe("Failed A");
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await change("Older own name"); await tick();
  expect(save).toHaveBeenCalledTimes(3);
  expect(save).toHaveBeenLastCalledWith("Older own name");
});

it.each([false, true])("does not replace owned preedit with an idle external update (interrupted=%s)", async interrupted => {
  const owner = new NameDraftOwner();
  await renderOwned(owner);
  await act(() => input().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
  if (interrupted) {
    await renderOwned(owner, "first", {}, false);
    await act(() => owner.sync([{ id: "first", name: "Newer B", provider: "alibabaCloud", credentialState: "present" }]));
  }
  await renderOwned(owner, "first", { value: "Newer B" });
  expect(input().value).toBe("Original");
  await tick(); expect(save).not.toHaveBeenCalled();
  if (interrupted) await key("Enter");
  else await act(() => input().dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
  await tick(); expect(save).toHaveBeenCalledExactlyOnceWith("Original");
});

it("keeps a failed owned draft and explicit retry after a newer pending-save source arrives", async () => {
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await render(); await change("Failed A"); await tick();
  await render({ value: "Newer B" });
  await act(async () => pending.reject(new Error("private-write-failure")));
  expect(input().value).toBe("Failed A");
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await tick(); expect(save).toHaveBeenCalledOnce();
  await act(async () => host.querySelector<HTMLButtonElement>('.auto-save-name-field__error button')!.click());
  expect(save).toHaveBeenCalledTimes(2);
  expect(input().value).toBe("Failed A");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("saves a new return to the initial name after failure against a newer authoritative baseline", async () => {
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await render(); await change("Failed A"); await tick();
  await render({ value: "Newer B" });
  await act(async () => pending.reject(new Error("private-write-failure")));
  expect(input().value).toBe("Failed A");
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await change("Original"); await tick();
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith("Original");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("keeps retry available when a current own acknowledgement preceded the command failure", async () => {
  const pending = deferred(); save.mockReturnValueOnce(pending.promise);
  await render(); await change("Failed A"); await tick();
  await render({ value: "External earlier" }); await render({ value: "Failed A" });
  await act(async () => pending.reject(new Error("private-write-failure")));
  expect(input().value).toBe("Failed A");
  await act(async () => host.querySelector<HTMLButtonElement>('.auto-save-name-field__error button')!.click());
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith("Failed A");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it.each(["success", "failure"] as const)("coalesces a remounted owner's newer edit after late %s without changing another profile", async outcome => {
  const owner = new NameDraftOwner(), first = deferred(), latest = deferred();
  const otherSave = vi.fn().mockResolvedValue({});
  save.mockReturnValueOnce(first.promise).mockReturnValueOnce(latest.promise);
  await renderOwned(owner); await change("First request"); await tick();
  await renderOwned(owner, "other", { value: "Other profile", onSave: otherSave });
  expect(input().value).toBe("Other profile");
  await renderOwned(owner);
  expect(input().value).toBe("First request");
  await change("Latest request"); await tick();
  expect(save).toHaveBeenCalledTimes(1);
  await act(async () => outcome === "success" ? first.resolve({}) : first.reject(new Error("private-old-failure")));
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith("Latest request");
  expect(input().value).toBe("Latest request");
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await renderOwned(owner, "other", { value: "Other profile", onSave: otherSave });
  await act(async () => latest.resolve({}));
  expect(input().value).toBe("Other profile");
  expect(otherSave).not.toHaveBeenCalled();
  await renderOwned(owner);
  expect(input().value).toBe("Latest request");
  expect(save).toHaveBeenCalledTimes(2);
});

it("preserves a late success until source acknowledgement and accepts the next external update while offscreen", async () => {
  const owner = new NameDraftOwner(), pending = deferred();
  save.mockReturnValueOnce(pending.promise);
  await renderOwned(owner); await change("Saved name");
  await renderOwned(owner, "first", {}, false);
  await act(async () => pending.resolve({}));
  await renderOwned(owner);
  expect(input().value).toBe("Saved name");
  await renderOwned(owner, "first", {}, false);
  await act(async () => owner.sync([{ id: "first", name: "Saved name", provider: "alibabaCloud", credentialState: "present" }]));
  await renderOwned(owner, "first", { value: "External name" });
  expect(input().value).toBe("External name");
  expect(save).toHaveBeenCalledExactlyOnceWith("Saved name");
});

it("discards a deleted owner's queued write and isolates late failure from its replacement", async () => {
  const owner = new NameDraftOwner(), pending = deferred();
  save.mockReturnValueOnce(pending.promise);
  await renderOwned(owner); await change("Pending old name"); await tick();
  await change("Queued old name");
  await renderOwned(owner, "first", {}, false);
  owner.discard("first");
  await renderOwned(owner, "first", { value: "Replacement profile" });
  await act(async () => pending.reject(new Error("private-deleted-profile-failure")));
  expect(input().value).toBe("Replacement profile");
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(save).toHaveBeenCalledExactlyOnceWith("Pending old name");
  await change("Replacement edit"); await tick();
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith("Replacement edit");
});

it("releases a failed draft explicitly replaced with the confirmed value without retrying it", async () => {
  const owner = new NameDraftOwner();
  save.mockRejectedValueOnce(new Error("private-failure"));
  await renderOwned(owner); await change("Discarded name"); await tick();
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await change("Original");
  await renderOwned(owner, "first", {}, false);
  await renderOwned(owner, "first", { value: "External name" });
  expect(input().value).toBe("External name");
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(save).toHaveBeenCalledExactlyOnceWith("Discarded name");
});

it("keeps interrupted IME preedit across navigation without saving it before a new edit or explicit confirmation", async () => {
  const owner = new NameDraftOwner();
  await renderOwned(owner);
  await act(() => input().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
  await change("wei"); await tick();
  await renderOwned(owner, "first", {}, false);
  expect(save).not.toHaveBeenCalled();
  await renderOwned(owner); await tick();
  expect(input().value).toBe("wei");
  expect(save).not.toHaveBeenCalled();
  await change("New committed edit"); await tick();
  expect(save).toHaveBeenCalledExactlyOnceWith("New committed edit");
  await act(() => input().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
  await change("plain");
  await renderOwned(owner, "first", {}, false);
  await renderOwned(owner);
  expect((await key("Enter")).defaultPrevented).toBe(true);
  expect(save).toHaveBeenLastCalledWith("plain");
});

it("quietly debounces repeated edits while preserving focus, raw spaces, punctuation and emoji", async () => {
  await render(); await act(() => input().focus());
  await change("  B 站 / 测试 · 😀  ");
  await act(() => input().setSelectionRange(3, 3));
  const original = input();
  await tick();
  expect(save).toHaveBeenCalledExactlyOnceWith("B 站 / 测试 · 😀");
  expect(input()).toBe(original);
  expect(input().value).toBe("  B 站 / 测试 · 😀  ");
  expect(document.activeElement).toBe(original);
  expect(input().selectionStart).toBe(3);
  expect(input().disabled).toBe(false);
  expect(host.querySelector('[role="status"], .settings-toast')).toBeNull();
  expect(host.querySelector('.auto-save-name-field button.settings-button')).toBeNull();
  await change("  B 站 / 第二次！😀  "); await tick();
  expect(save).toHaveBeenLastCalledWith("B 站 / 第二次！😀");
  expect(input().value).toBe("  B 站 / 第二次！😀  ");
  expect(document.activeElement).toBe(original);
});

it("serializes writes and coalesces newer drafts without disabling the current edit", async () => {
  const first = deferred(), second = deferred();
  save.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  await render(); await change("First"); await tick();
  await change("Intermediate"); await tick();
  await change("  Latest  "); await tick();
  expect(save).toHaveBeenCalledTimes(1);
  expect(input().disabled).toBe(false);
  await render({ value: "First" });
  expect(input().value).toBe("  Latest  ");
  await act(async () => first.resolve({}));
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith("Latest");
  await act(async () => second.resolve({}));
  expect(input().value).toBe("  Latest  ");
  await render({ value: "Latest" });
  expect(input().value).toBe("  Latest  ");
});

it("ignores the first acknowledgement arriving after another edit or completed save", async () => {
  await render(); await change("First"); await tick();
  await change("  Second  ");
  await render({ value: "First" });
  expect(input().value).toBe("  Second  ");
  await tick();
  expect(save).toHaveBeenLastCalledWith("Second");
  await render({ value: "Second" });
  expect(input().value).toBe("  Second  ");
  await change("Third"); await tick();
  await change("Fourth"); await tick();
  await render({ value: "Third" });
  expect(input().value).toBe("Fourth");
  await render({ value: "Fourth" });
  await render({ value: "First" }); // A later external edit may reuse an acknowledged old name.
  expect(input().value).toBe("First");
});

it("flushes Enter and external blur without submitting credentials or changing the input geometry", async () => {
  await render(); await act(() => input().focus());
  await change("Enter name");
  expect((await key("Enter")).defaultPrevented).toBe(true);
  expect(save).toHaveBeenCalledExactlyOnceWith("Enter name");
  expect(submit).not.toHaveBeenCalled();
  await change("Expanded name");
  expect(host.querySelector(".config-input__expand")).toBeNull();
  expect(input().tagName).toBe("INPUT");
  expect(save).toHaveBeenCalledTimes(1);
  expect(document.activeElement).toBe(input());
  await act(async () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!.focus());
  expect(save).toHaveBeenLastCalledWith("Expanded name");
  await change("Expanded Enter"); await key("Enter");
  expect(save).toHaveBeenLastCalledWith("Expanded Enter");
  expect(submit).not.toHaveBeenCalled();
});

it("waits for composition to finish and leaves native IME Enter alone including keyCode 229", async () => {
  await render();
  await act(() => input().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
  await change("b"); await tick();
  expect((await key("Enter", { isComposing: true })).defaultPrevented).toBe(false);
  expect((await key("Enter", { keyCode: 229 })).defaultPrevented).toBe(false);
  expect(save).not.toHaveBeenCalled();
  await change("B 站");
  await act(() => input().dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "站" })));
  expect((await key("Enter", { keyCode: 229 })).defaultPrevented).toBe(false);
  expect(save).not.toHaveBeenCalled();
  await tick();
  expect(save).toHaveBeenCalledExactlyOnceWith("B 站");
  expect(submit).not.toHaveBeenCalled();
});

it("retains a failed draft, sanitizes errors and retries only when requested or edited", async () => {
  save.mockRejectedValueOnce(new Error("synthetic-private-path"));
  await render(); await change("Unsaved"); await tick();
  expect(input().value).toBe("Unsaved");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.settings.profileActionFailed);
  expect(host.textContent).not.toContain("synthetic-private-path");
  await tick(); await key("Enter");
  expect(save).toHaveBeenCalledTimes(1);
  await act(async () => host.querySelector<HTMLButtonElement>('.auto-save-name-field__error button')!.click());
  expect(save).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(input().value).toBe("Unsaved");
  save.mockResolvedValueOnce(null);
  await change("Rejected"); await tick();
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await change("Edited retry"); await tick();
  expect(save).toHaveBeenLastCalledWith("Edited retry");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("does not let a failed earlier request replace or report an error on a newer draft", async () => {
  const first = deferred(); save.mockReturnValueOnce(first.promise);
  await render(); await change("First"); await tick();
  await change("Second"); await tick();
  await act(async () => first.reject(new Error("private-first-failure")));
  expect(save).toHaveBeenLastCalledWith("Second");
  expect(input().value).toBe("Second");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("accepts an external update reusing a failed name after the focused field saves a newer edit", async () => {
  save.mockRejectedValueOnce(new Error("synthetic-save-failure"));
  await render(); await change("Failed name"); await tick();
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await change("Good name"); await tick();
  await render({ value: "Good name" });
  expect(input().value).toBe("Good name");
  expect(document.activeElement).toBe(input());
  await render({ value: "Failed name" });
  expect(input().value).toBe("Failed name");
  expect(document.activeElement).toBe(input());
  expect(save).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("flushes the last draft on a keyed switch without changing the replacement editor", async () => {
  const first = deferred(); const otherSave = vi.fn().mockResolvedValue({});
  save.mockReturnValueOnce(first.promise);
  await render(); await change("First"); await tick();
  await change("Last old name");
  await render({ value: "Other profile", onSave: otherSave }, "second");
  expect(input().value).toBe("Other profile");
  await act(async () => first.resolve({}));
  expect(save).toHaveBeenLastCalledWith("Last old name");
  expect(save).toHaveBeenCalledTimes(2);
  expect(otherSave).not.toHaveBeenCalled();
  expect(input().value).toBe("Other profile");
  await change("Second final draft");
  await act(() => root.render(<div>Closed</div>));
  expect(otherSave).toHaveBeenCalledExactlyOnceWith("Second final draft");
});

it("finishes an owned draft after another action disables and unmounts its editor", async () => {
  const first = deferred(); const replacementSave = vi.fn().mockResolvedValue({});
  save.mockReturnValueOnce(first.promise);
  await render(); await change("First"); await tick();
  await change("Latest owned draft");
  await render({ disabled: true });
  expect(input().disabled).toBe(true);
  await render({ value: "Replacement", onSave: replacementSave }, "replacement");
  await act(async () => first.resolve({}));
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith("Latest owned draft");
  expect(replacementSave).not.toHaveBeenCalled();
  expect(input().value).toBe("Replacement");
});

it("validates an empty required profile name but permits clearing a translation name", async () => {
  await render({ allowEmpty: false }); await change("   "); await tick();
  expect(save).not.toHaveBeenCalled();
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.nameRequired);
  expect(host.querySelector('.auto-save-name-field__error button')).toBeNull();
  await change("Restored"); await tick();
  expect(save).toHaveBeenCalledExactlyOnceWith("Restored");
  await render({}, "optional"); await change("   "); await tick();
  expect(save).toHaveBeenLastCalledWith("");
  expect(input().value).toBe("   ");
});

it("accepts an external name when clean and keeps disabled and read-only fields unchanged", async () => {
  await render(); await render({ value: "Updated elsewhere" });
  expect(input().value).toBe("Updated elsewhere");
  await render({ value: "Updated elsewhere", disabled: true });
  expect(input().disabled).toBe(true);
  await key("Enter"); expect(save).not.toHaveBeenCalled();
  await render({ value: "Updated elsewhere", readOnly: true });
  expect(input().readOnly).toBe(true);
  await key("Enter"); expect(save).not.toHaveBeenCalled();
});

it("accepts a reused external name while idle even when its earlier save had no prop echo", async () => {
  await render(); await change("First"); await tick();
  await change("Second"); await tick();
  await act(async () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!.focus());
  await render({ value: "First" });
  expect(input().value).toBe("First");
  expect(save).toHaveBeenCalledTimes(2);
});

it("keeps names single-line without a misleading expansion affordance", async () => {
  await render();
  expect(host.querySelector(".config-input__expand")).toBeNull();
  await change("  A / B 😀  "); await tick();
  expect(input().value).toBe("  A / B 😀  ");
  expect(save).toHaveBeenCalledExactlyOnceWith("A / B 😀");
});
