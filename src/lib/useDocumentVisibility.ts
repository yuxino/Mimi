import { useSyncExternalStore } from "react";

function subscribe(listener: () => void) {
  document.addEventListener("visibilitychange", listener);
  return () => document.removeEventListener("visibilitychange", listener);
}

function visible() { return !document.hidden; }

/** A visible, unfocused window still needs current presentation data. Native
 * WebView teardown uses each consumer's ordinary effect cleanup. */
export function useDocumentVisibility() {
  return useSyncExternalStore(subscribe, visible);
}
