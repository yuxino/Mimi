import { readdir, readFile } from "node:fs/promises";
import { basename, join, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const forbidden = /development_debug_|development-debug-flush|development-debugger|DevelopmentDebugger|DevelopmentOverlayTrace|developmentTrace|debugSnapshotId|data-debug-lane/;

/** Check every emitted file, including lazy chunks, CSS and source maps. */
export async function checkProductionDebugger(directory) {
  const root = resolve(directory);
  let checked = 0;
  async function visit(current) {
    for (const entry of await readdir(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) {
        await visit(path);
      } else if (entry.isFile()) {
        const content = await readFile(path);
        if (forbidden.test(basename(path)) || forbidden.test(content.toString("utf8"))) {
          throw new Error(`Production contains development debugger code: ${relative(root, path)}`);
        }
        checked += 1;
      }
    }
  }
  await visit(root);
  if (checked === 0) throw new Error("Production output is empty");
  return checked;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const checked = await checkProductionDebugger(process.argv[2] ?? "dist");
    console.log(`Production debugger boundary verified (${checked} files).`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
