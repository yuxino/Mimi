import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./ipc";

/** Explicit settings paste only. Never poll, log or persist clipboard contents. */
export function readClipboardText(): Promise<string> {
  return isTauri ? invoke<string>("plugin:clipboard-manager|read_text") : navigator.clipboard.readText();
}
