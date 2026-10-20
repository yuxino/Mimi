import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { startFrontendServer } from "./dev-app-watch.mjs";

test("signed watch preview serves the configured React entry and development flags", async () => {
  const server = await startFrontendServer();
  try {
    const index = await readFile(new URL("../index.html", import.meta.url), "utf8");
    const html = await server.transformIndexHtml("/", index);
    assert.match(html, /@react-refresh/);
    assert.match(html, /\/src\/main\.tsx/);
    assert.equal(server.config.define.__MIMI_DEVELOPMENT_BUILD__, "true");
    assert.equal(server.config.envDir, false);
  } finally {
    await server.close();
  }
});
