import { appIsLinuxPackage, appIsPortable, appIsUiTest, isTauri } from "../../lib/ipc";
import {
  createFixtureSoftwareUpdater,
  createTauriSoftwareUpdater,
  isWindowsUserAgent,
  type SoftwareUpdater,
} from "./softwareUpdater";

type UpdateEnvironment =
  | { kind: "portable" | "linuxPackage"; currentVersion: string }
  | { kind: "installed"; updater: SoftwareUpdater };

export async function createUpdaterForEnvironment(): Promise<UpdateEnvironment> {
  if (!isTauri) {
    return {
      kind: "installed",
      updater: createFixtureSoftwareUpdater({
        currentVersion: "preview",
        updateVersion: null,
      }),
    };
  }

  if (isWindowsUserAgent()) {
    let portable = true;
    try {
      portable = await appIsPortable();
    } catch {
      // A failed mode check must not offer an installer to a portable copy.
    }
    if (portable) {
      const { getVersion } = await import("@tauri-apps/api/app");
      return { kind: "portable", currentVersion: await getVersion() };
    }
  }

  // Native distribution detection avoids guessing from the WebView user agent.
  // If detection fails, initialization fails closed before creating an updater.
  if (await appIsLinuxPackage()) {
    const { getVersion } = await import("@tauri-apps/api/app");
    return { kind: "linuxPackage", currentVersion: await getVersion() };
  }

  if (await appIsUiTest()) {
    const { getVersion } = await import("@tauri-apps/api/app");
    return {
      kind: "installed",
      updater: createFixtureSoftwareUpdater({ currentVersion: await getVersion() }),
    };
  }

  return { kind: "installed", updater: await createTauriSoftwareUpdater() };
}
