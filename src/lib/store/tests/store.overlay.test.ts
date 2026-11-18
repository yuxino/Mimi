import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useStore } from "../../store";

const native = vi.hoisted(() => ({ setCollapsed: vi.fn() }));
vi.mock("../../ipc", async (original) => ({
  ...await original<typeof import("../../ipc")>(),
  isTauri: true,
  overlaySetCollapsed: native.setCollapsed,
}));

const initial = useStore.getState();
beforeEach(() => {
  native.setCollapsed.mockReset();
  useStore.setState(initial, true);
});
afterEach(() => useStore.setState(initial, true));

it("restores expanded presentation after a rejected optimistic collapse without removing new subtitles", async () => {
  let reject!: (error: Error) => void;
  native.setCollapsed.mockImplementation(() => new Promise<void>((_resolve, fail) => { reject = fail; }));
  const collapse = useStore.getState().setOverlayCollapsed(true);
  expect(useStore.getState().session.isOverlayCollapsed).toBe(true);
  const subtitles = { ...initial.session.subtitles, source: { text: "Synthetic newer draft.", isFinal: false } };
  useStore.setState(state => ({ session: { ...state.session, subtitles } }));
  const failed = expect(collapse).rejects.toThrow("synthetic-collapse-failure");
  reject(new Error("synthetic-collapse-failure")); await failed;
  expect(useStore.getState().session.isOverlayCollapsed).toBe(false);
  expect(useStore.getState().session.subtitles).toBe(subtitles);
});

it("does not roll a newer successful collapse back when an older request fails", async () => {
  let reject!: (error: Error) => void;
  native.setCollapsed.mockImplementationOnce(() => new Promise<void>((_resolve, fail) => { reject = fail; }));
  native.setCollapsed.mockResolvedValueOnce(undefined);
  const first = useStore.getState().setOverlayCollapsed(true);
  await useStore.getState().setOverlayCollapsed(true);
  const failed = expect(first).rejects.toThrow("synthetic-old-failure");
  reject(new Error("synthetic-old-failure")); await failed;
  expect(useStore.getState().session.isOverlayCollapsed).toBe(true);
});

it("leaves a newer native collapsed state intact after an expansion rejects", async () => {
  useStore.setState(state => ({ session: { ...state.session, isOverlayCollapsed: true } }));
  let reject!: (error: Error) => void;
  native.setCollapsed.mockImplementation(() => new Promise<void>((_resolve, fail) => { reject = fail; }));
  const expand = useStore.getState().setOverlayCollapsed(false);
  useStore.setState(state => ({ session: { ...state.session, isOverlayCollapsed: true } }));
  const failed = expect(expand).rejects.toThrow("synthetic-expansion-failure");
  reject(new Error("synthetic-expansion-failure")); await failed;
  expect(useStore.getState().session.isOverlayCollapsed).toBe(true);
});
