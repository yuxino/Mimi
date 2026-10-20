#!/usr/bin/env node

import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { createServer } from "vite";

const projectDirectory = fileURLToPath(new URL("../", import.meta.url));
const frontendUrl = "http://127.0.0.1:1420";

export async function startFrontendServer() {
  const server = await createServer({
    configFile: resolve(projectDirectory, "config/vite.config.ts"),
    root: projectDirectory,
    envDir: false,
    // QA fixtures and old Android reports are not application entry points.
    optimizeDeps: { entries: ["index.html"] },
    server: {
      host: "127.0.0.1",
      port: 1420,
      strictPort: true,
      hmr: { protocol: "ws", host: "127.0.0.1", port: 1420 },
      watch: {
        ignored: ["**/src-tauri/**", "**/android/**", "**/.cargo-home/**", "**/.npm-cache/**"],
      },
    },
  });
  try {
    await server.listen();
    return server;
  } catch (error) {
    await server.close();
    throw error;
  }
}

export async function watchDevelopment(args) {
  let server;
  let installer;
  let stopSignal;
  let requestStop;
  const stopped = new Promise((resolveStop) => { requestStop = resolveStop; });
  const signalExitCode = () => stopSignal === "SIGINT" ? 130 : 143;
  const stop = (signal) => {
    if (stopSignal) return;
    stopSignal = signal;
    if (installer?.pid && installer.exitCode === null && installer.signalCode === null) {
      try {
        // Signal only the group created below, including its active compiler.
        // Awaiting close lets the installer's EXIT trap restore/clean staging.
        process.kill(-installer.pid, signal);
      } catch (error) {
        if (error.code !== "ESRCH") console.error(`error: ${error.message}`);
      }
    }
    requestStop();
  };
  const interrupt = () => stop("SIGINT");
  const terminate = () => stop("SIGTERM");
  process.on("SIGINT", interrupt);
  process.on("SIGTERM", terminate);

  try {
    server = await startFrontendServer();
    if (stopSignal) return signalExitCode();
    console.log(`Development frontend: ${frontendUrl}`);
    installer = spawn("bash", [
      resolve(projectDirectory, "scripts/dev-app.sh"),
      ...args.filter((argument) => argument !== "--watch"),
    ], {
      cwd: projectDirectory,
      env: { ...process.env, MIMI_DEV_WATCH: "1" },
      stdio: "inherit",
      detached: true,
    });
    const result = await new Promise((resolveResult) => {
      installer.once("error", (error) => resolveResult({ error }));
      installer.once("close", (code, signal) => resolveResult({ code, signal }));
    });
    if (stopSignal) return signalExitCode();
    if (result.error) throw result.error;
    if (result.code !== 0) {
      console.error(`error: development install failed${result.signal ? ` (${result.signal})` : ""}.`);
      return result.code || 1;
    }
    console.log("React and CSS changes appear in Mimi when you save. Keep this terminal open.");
    console.log("After Rust or native configuration changes, press Ctrl-C and rerun the same command.");
    console.log("Ctrl-C stops this frontend server; quit Mimi before leaving the preview session.");
    await stopped;
    return signalExitCode();
  } finally {
    // Do not kill Mimi or any previously running Vite/app process.
    await server?.close();
    process.off("SIGINT", interrupt);
    process.off("SIGTERM", terminate);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = await watchDevelopment(process.argv.slice(2));
  } catch (error) {
    console.error(`error: ${error.message}`);
    process.exitCode = 1;
  }
}
