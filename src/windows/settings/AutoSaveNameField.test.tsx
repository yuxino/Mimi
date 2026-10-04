// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N } from "../../lib/i18n";
import { AutoSaveNameField } from "./AutoSaveNameField";

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

it("flushes Enter and external blur without submitting credentials or saving internal expansion", async () => {
  await render(); await act(() => input().focus());
  await change("Enter name");
  expect((await key("Enter")).defaultPrevented).toBe(true);
  expect(save).toHaveBeenCalledExactlyOnceWith("Enter name");
  expect(submit).not.toHaveBeenCalled();
  await change("Expanded name");
  await act(() => host.querySelector<HTMLButtonElement>('.config-input__expand')!.click());
  expect(input().tagName).toBe("TEXTAREA");
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

it("normalizes pasted line breaks in an expanded name while preserving other characters", async () => {
  await render();
  await act(() => host.querySelector<HTMLButtonElement>('.config-input__expand')!.click());
  await change("  A\r\n / B 😀  "); await tick();
  expect(input().value).toBe("  A / B 😀  ");
  expect(save).toHaveBeenCalledExactlyOnceWith("A / B 😀");
});
