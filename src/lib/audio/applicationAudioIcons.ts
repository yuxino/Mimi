import type { ApplicationSnapshot } from "./applicationAudio";

const PNG_PREFIX = "data:image/png;base64,";
const MAX_ICONS = 128;
const MAX_ICON_CHARACTERS = 12 * 1024;
const MAX_SNAPSHOT_CHARACTERS = 1024 * 1024;

/** One bounded snapshot of decorative icons; no application choices or disk state. */
export function createApplicationIconCache() {
  let icons: ReadonlyMap<string, string> = new Map();
  return {
    read: () => icons,
    clear: () => { icons = new Map(); },
    replace(snapshot: ApplicationSnapshot) {
      const next = new Map<string, string>();
      let characters = 0;
      if (snapshot.supported) for (const app of snapshot.applications) {
        const icon = app.iconDataUrl;
        if (!icon?.startsWith(PNG_PREFIX) || icon.length > MAX_ICON_CHARACTERS || next.has(app.id)) continue;
        if (next.size >= MAX_ICONS || characters + icon.length > MAX_SNAPSHOT_CHARACTERS) break;
        next.set(app.id, icon);
        characters += icon.length;
      }
      icons = next;
    },
  };
}

// Each Tauri WebView has its own module instance. Closing the floating panel
// unmounts its controls, but keeps this small cache until the window is reloaded.
export const applicationIconCache = createApplicationIconCache();
