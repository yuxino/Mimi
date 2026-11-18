import { expect, it, vi } from "vitest";
import { SubtitleFontChoiceOwner } from "./subtitleFontChoiceOwner";

const current = () => true;

function deferred() {
  let resolve!: (value?: unknown) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<unknown>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

it("keeps only the latest queued font while one save is unresolved", async () => {
  const owner = new SubtitleFontChoiceOwner();
  const first = deferred();
  const save = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(undefined);
  const failed = vi.fn();
  owner.choose("A", save, failed, current);
  for (let index = 0; index < 1_000; index++) owner.choose(`font-${index}`, save, failed, current);
  expect(save.mock.calls).toEqual([["A"]]);
  expect(owner.getSnapshot()).toBe("font-999");
  first.resolve();
  await vi.waitFor(() => expect(owner.getSnapshot()).toBeNull());
  expect(save.mock.calls).toEqual([["A"], ["font-999"]]);
  expect(failed).not.toHaveBeenCalled();
});

it.each([false, true])("deduplicates a successful ABA choice but retries it after failure=%s", async fails => {
  const owner = new SubtitleFontChoiceOwner();
  const first = deferred();
  const save = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(undefined);
  const failed = vi.fn();
  owner.choose("A", save, failed, current);
  owner.choose("B", save, failed, current);
  owner.choose("A", save, failed, current);
  if (fails) first.reject(new Error("synthetic")); else first.resolve();
  await vi.waitFor(() => expect(owner.getSnapshot()).toBeNull());
  expect(save.mock.calls).toEqual(fails ? [["A"], ["A"]] : [["A"]]);
  expect(failed).toHaveBeenCalledTimes(fails ? 1 : 0);
});

it.each([false, null])("reports a rejected save result (%s) and still persists a newer choice", async rejected => {
  const owner = new SubtitleFontChoiceOwner();
  const save = vi.fn().mockResolvedValueOnce(rejected).mockResolvedValue(undefined);
  const failed = vi.fn();
  owner.choose("A", save, failed, current);
  owner.choose("B", save, failed, current);
  await vi.waitFor(() => expect(owner.getSnapshot()).toBeNull());
  expect(save.mock.calls).toEqual([["A"], ["B"]]);
  expect(failed).toHaveBeenCalledOnce();
});

it("reserves one writer before notifying reentrant subscribers", async () => {
  const owner = new SubtitleFontChoiceOwner();
  const save = vi.fn().mockResolvedValue(undefined);
  const failed = vi.fn();
  let replaced = false;
  owner.subscribe(() => {
    if (!replaced && owner.getSnapshot() === "A") {
      replaced = true;
      owner.choose("B", save, failed, current);
    }
  });
  owner.choose("A", save, failed, current);
  await vi.waitFor(() => expect(owner.getSnapshot()).toBeNull());
  expect(save.mock.calls).toEqual([["B"]]);
});
