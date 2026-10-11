import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./ipc";

/** Explicit settings paste only. Never poll, log or persist clipboard contents. */
export function readClipboardText(): Promise<string> {
  return isTauri ? invoke<string>("plugin:clipboard-manager|read_text") : navigator.clipboard.readText();
}

/** Explicit copy of an opted-in configuration value; never read stored secrets. */
export function writeClipboardText(text: string): Promise<void> {
  return isTauri ? invoke<void>("plugin:clipboard-manager|write_text", { text }) : navigator.clipboard.writeText(text);
}
