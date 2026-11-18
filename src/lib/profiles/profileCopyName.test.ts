import { expect, it } from "vitest";
import { profileCopyName } from "./profileCopyName";

it("keeps a unique localized suffix within the native Unicode limit", () => {
  const name = "字幕🌸".repeat(24);
  const first = profileCopyName(name, "副本", []);
  const second = profileCopyName(name, "副本", [{ name: first }]);
  expect(Array.from(first)).toHaveLength(64);
  expect(Array.from(second)).toHaveLength(64);
  expect(first.endsWith(" (副本)")).toBe(true);
  expect(second.endsWith(" (副本 2)")).toBe(true);
  expect(second).not.toContain("\uFFFD");
});
