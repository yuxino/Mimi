import { defineConfig, normalizePath } from "vite";
import react from "@vitejs/plugin-react";

// `process` is provided by Node when Vite loads this config; the ambient
// declaration keeps the type check self-contained (no @types/node dependency).
declare const process: {
  env: Record<string, string | undefined>;
  platform: string;
};
// Vite preserves the original config directory when bundling this file.
declare const __dirname: string;

const host = process.env.TAURI_DEV_HOST;
const tauriPlatform = process.env.TAURI_ENV_PLATFORM;
const buildPlatform = tauriPlatform ?? process.platform;

// Match the JavaScript output to the native runtime baselines. macOS 13 ships
// Safari 16; current Tauri Windows builds require WebView2/Chromium 105.
const buildTarget =
  buildPlatform === "windows" || buildPlatform === "win32"
    ? "chrome105"
    : buildPlatform === "darwin"
      ? "safari16"
      : "es2021";

// https://vitejs.dev/config/
export default defineConfig(({ command, mode }) => {
  // `vite build --mode development` intentionally keeps the local debugger.
  // A release build never resolves its UI, replay or observation modules.
  const developmentBuild = command === "serve" || mode === "development";
  const productionAdapter = `${normalizePath(__dirname)}/src/lib/productionDevelopmentTools.ts`;
  return {
    plugins: [react()],
    define: { __MIMI_DEVELOPMENT_BUILD__: JSON.stringify(developmentBuild) },
    resolve: {
      alias: developmentBuild ? [] : [
        { find: /^(?:.*\/)?DevelopmentDebugger(?:\.tsx)?$/, replacement: productionAdapter },
        { find: /^(?:.*\/)?DevelopmentOverlayTrace(?:\.tsx)?$/, replacement: productionAdapter },
        { find: /^(?:.*\/)?developmentTrace(?:\.ts)?$/, replacement: productionAdapter },
      ],
    },
    build: {
      target: buildTarget,
    },

    // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
    //
    // 1. prevent vite from obscuring rust errors
    clearScreen: false,
    // 2. tauri expects a fixed port, fail if that port is not available
    server: {
      port: 1420,
      strictPort: true,
      host: host || false,
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 1421,
          }
        : undefined,
      watch: {
        // 3. tell vite to ignore watching `src-tauri`
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
