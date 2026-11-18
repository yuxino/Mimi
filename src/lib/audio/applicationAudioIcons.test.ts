import { expect, it } from "vitest";
import { createApplicationIconCache } from "./applicationAudioIcons";

const png = "data:image/png;base64,c3ludGhldGlj";
it("keeps only embedded PNG icons from the latest supported snapshot", () => {
  const cache = createApplicationIconCache();
  cache.replace({ supported: true, applications: [
    { id: "player", name: "Player", iconDataUrl: png },
    { id: "remote", name: "Remote", iconDataUrl: "https://example.invalid/icon.png" },
    { id: "empty", name: "Empty", iconDataUrl: null },
  ] });
  expect([...cache.read()]).toEqual([["player", png]]);
  cache.replace({ supported: true, applications: [] });
  expect(cache.read().size).toBe(0);
  cache.replace({ supported: false, applications: [{ id: "player", name: "Player", iconDataUrl: png }] });
  expect(cache.read().size).toBe(0);
});

it("bounds both retained icon count and total image size", () => {
  const cache = createApplicationIconCache();
  const applications = Array.from({ length: 300 }, (_, id) => ({ id: `${id}`, name: `${id}`, iconDataUrl: png }));
  cache.replace({ supported: true, applications });
  expect(cache.read().size).toBe(128);
  cache.replace({ supported: true, applications: applications.map(app => ({ ...app, iconDataUrl: png + "a".repeat(10 * 1024) })) });
  expect([...cache.read().values()].reduce((size, icon) => size + icon.length, 0)).toBeLessThanOrEqual(1024 * 1024);
  expect(cache.read().size).toBeLessThan(128);
  cache.replace({ supported: true, applications: [{ id: "huge", name: "Huge", iconDataUrl: png + "a".repeat(12 * 1024) }] });
  expect(cache.read().size).toBe(0);
});
