import { useEffect, useState } from "react";
import { appDesktopShortcutCommands, isTauri, type DesktopShortcutCommands } from "./ipc";

type ShortcutSupport = DesktopShortcutCommands | null;
// Native discovery depends only on this process's platform and installation.
// Module state belongs to one WebView; every other window verifies its own IPC.
let cachedCommands: ShortcutSupport | undefined;
let pendingRequest: Promise<ShortcutSupport> | undefined;

function readShortcutSupport(): Promise<ShortcutSupport> {
  if (cachedCommands !== undefined) return Promise.resolve(cachedCommands);
  if (pendingRequest) return pendingRequest;
  pendingRequest = Promise.resolve().then(appDesktopShortcutCommands).then(value => {
    cachedCommands = value;
    return value;
  }).finally(() => { pendingRequest = undefined; });
  return pendingRequest;
}

/** Undefined while unknown: never advertise an unverified X11 binding on Wayland. */
export function useDesktopShortcuts() {
  const [commands, setCommands] = useState<DesktopShortcutCommands | null | undefined>(
    () => isTauri ? cachedCommands : null,
  );
  useEffect(() => {
    if (!isTauri) return;
    let active = true;
    void readShortcutSupport().then((value) => {
      if (active) setCommands(value);
    }).catch(() => {
      // Keep ordinary buttons available without claiming native shortcut support.
    });
    return () => { active = false; };
  }, []);
  return { commands, nativeShortcuts: commands === null };
}
