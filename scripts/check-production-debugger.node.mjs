import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { checkProductionDebugger } from "./check-production-debugger.mjs";

async function output(t) {
  const directory = await mkdtemp(join(tmpdir(), "mimi-production-boundary-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(join(directory, "assets"));
  return directory;
}

test("accepts normal settings, diagnostics and subtitle IPC", async t => {
  const directory = await output(t);
  await writeFile(join(directory, "index.html"), '<script src="/assets/app.js"></script>');
  await writeFile(join(directory, "assets", "app.js"), 'invoke("session_get_state"); invoke("diagnostics_snapshot");');
  assert.equal(await checkProductionDebugger(directory), 2);
});

for (const [filename, content] of [
  ["app.js", 'invoke("development_debug_snapshot")'],
  ["overlay.js", 'listen("development-debug-flush")'],
  ["chunk.css", ".development-debugger{display:block}"],
  ["app.js.map", '{"sources":["../src/lib/developmentTrace.ts"]}'],
  ["DevelopmentDebugger-123.js", "export{}"],
  ["overlay.js", 'element["data-debug-lane"]'],
]) {
  test(`rejects leaked developer artifact ${filename}: ${content}`, async t => {
    const directory = await output(t);
    await writeFile(join(directory, "assets", filename), content);
    await assert.rejects(checkProductionDebugger(directory), /Production contains development debugger code/);
  });
}

test("rejects empty or absent output rather than reporting a false pass", async t => {
  const directory = await output(t);
  await assert.rejects(checkProductionDebugger(directory), /Production output is empty/);
  await assert.rejects(checkProductionDebugger(join(directory, "missing")), { code: "ENOENT" });
});
