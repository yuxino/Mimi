/**
 * Thin wrappers over the Tauri IPC surface defined by the contract in
 * `docs/plans/2026-08-22-multi-provider-professional-settings-design.md`.
 * Command names and payload
 * keys are fixed by that contract and must not drift.
 */

import { invoke } from "@tauri-apps/api/core";
import { observeSessionWireReceived } from "./developmentTrace";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ProfileNetworkProxyDraft,
  AudioInput,
  AudioSource,
  ProviderCredentialsInput,
  SessionStateEvent,
  SessionArchiveState,
  SessionExportKind,
  TranscriptPage,
  SessionHistoryItem,
  ServiceProvider,
  SettingsDraft,
  SettingsSnapshot,
  SourceLanguage,
  SystemAudioTarget,
  TargetLanguage,
  TextTranslation,
  TranslationMode,
} from "./types";

/**
 * Whether the app is running inside Tauri. In a plain `vite dev` session there
 * are no `__TAURI_INTERNALS__`, so the store falls back to mock behavior.
 */
export const isTauri =
  typeof window !== "undefined" &&
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !==
    undefined;

// ---------------------------------------------------------------------------
// Commands (frontend -> Rust)
// ---------------------------------------------------------------------------

export function sessionStart(): Promise<void> {
  return invoke("session_start");
}

export function sessionStop(): Promise<void> {
  return invoke("session_stop");
}

export function sessionTogglePaused(): Promise<void> {
  return invoke("session_toggle_paused");
}

export function sessionClearSubtitles(): Promise<void> {
  return invoke("session_clear_subtitles");
}

export function sessionSwitchSourceLanguage(
  language: SourceLanguage,
): Promise<void> {
  return invoke("session_switch_source_language", { language });
}

export function sessionSwitchAudioInput(input: AudioInput): Promise<void> {
  return invoke("session_switch_audio_input", { input });
}

export function sessionSwitchSystemAudioTarget(target: SystemAudioTarget): Promise<void> {
  return invoke("session_switch_system_audio_target", { target });
}

export function sessionSwitchTargetLanguage(language: TargetLanguage): Promise<void> {
  return invoke("session_switch_target_language", { language });
}

export function sessionSwitchTranslationMode(
  mode: TranslationMode,
): Promise<void> {
  return invoke("session_switch_translation_mode", { mode });
}

export function installedFontFamilies(): Promise<string[]> {
  return isTauri ? invoke<string[]>("installed_font_families") : Promise.resolve([]);
}

export function settingsGet(): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>("settings_get");
}

export function appIsUiTest(): Promise<boolean> {
  return invoke<boolean>("app_is_ui_test");
}

export function appUiTestFrontendReady(): Promise<void> {
  return invoke("app_ui_test_frontend_ready");
}

export function appIsPortable(): Promise<boolean> {
  return invoke<boolean>("app_is_portable");
}

export function appIsLinuxPackage(): Promise<boolean> {
  return invoke<boolean>("app_is_linux_package");
}

export function appOpenReleases(): Promise<void> {
  return invoke("app_open_releases");
}

export function settingsSave(draft: SettingsDraft): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>("settings_save", { draft });
}

export function profileCreate(
  provider: ServiceProvider,
  name: string,
): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>("profile_create", { provider, name });
}

export function profileUpdate(
  profileId: string,
  name: string,
  proxies?: ProfileNetworkProxyDraft,
): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>("profile_update", { profileId, name, ...proxies });
}

export function profileSelect(profileId: string): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>("profile_select", { profileId });
}

export function profileDelete(profileId: string): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>("profile_delete", { profileId });
}

export function profileSaveCredentials(
  profileId: string,
  credentials: ProviderCredentialsInput,
): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>("profile_save_credentials", {
    profileId,
    credentials,
  });
}

export function profileDeleteAPIKey(
  profileId: string,
): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>("profile_delete_api_key", { profileId });
}

export type StoredCredentialField = "apiKey" | "asrApiKey" | "token" | "secretId" | "secretKey" | "appKey";

/** Settings-only, explicit user reveal. Never includes secrets in a snapshot. */
export function profileRevealCredential(request: {
  profileId: string;
  field: StoredCredentialField;
  textTranslation?: Exclude<TextTranslation, "followService">;
}): Promise<string | null> {
  return invoke<string | null>("profile_reveal_credential", request);
}

export function overlaySetCollapsed(collapsed: boolean): Promise<void> {
  return invoke("overlay_set_collapsed", { collapsed });
}

export function overlaySetLocked(locked: boolean): Promise<void> {
  return invoke("overlay_set_locked", { locked });
}

export function overlayShow(): Promise<void> {
  return invoke("overlay_show");
}

export function overlayMoveStart(): Promise<void> {
  return invoke("overlay_move_start");
}

export type OverlayControlMode = "hidden" | "island" | "panel";

/** Toggles the child overlay control between its compact island and panel.
 * The native command keeps its established name for backwards compatibility. */
export function overlayPopoverToggle(): Promise<void> {
  return invoke("overlay_popover_toggle");
}

/** Returns the expanded child control to its compact island. */
export function overlayPopoverHide(): Promise<void> {
  return invoke("overlay_popover_hide");
}

export function overlayControlGetState(): Promise<OverlayControlMode> {
  return invoke<OverlayControlMode>("overlay_control_state");
}

export function overlayControlSetPanelHeight(height: number): Promise<void> {
  return invoke("overlay_control_set_panel_height", { height });
}

export function overlayControlSetIslandWidth(width: number): Promise<void> {
  return invoke("overlay_control_set_island_width", { width });
}

/** Fetches the current session state snapshot (for windows that boot after
 * the last session-state broadcast). */
export function sessionGetState(): Promise<SessionStateEvent> {
  return invoke<SessionStateEvent>("session_get_state").then(state => {
    if (__MIMI_DEVELOPMENT_BUILD__) observeSessionWireReceived(state);
    return state;
  });
}

export function trayPanelHide(): Promise<void> {
  return invoke("tray_panel_hide");
}

export function appQuit(): Promise<void> {
  return invoke("app_quit");
}

export type SettingsNavigationTarget = "service" | "export";

export function appShowSettings(
  target?: SettingsNavigationTarget,
): Promise<void> {
  return invoke("app_show_settings", { target: target ?? null });
}

// ---------------------------------------------------------------------------
// Events (Rust -> frontend)
// ---------------------------------------------------------------------------

export function listenSessionState(
  handler: (state: SessionStateEvent) => void,
): Promise<UnlistenFn> {
  return listen<SessionStateEvent>("session-state", (event) => {
    if (__MIMI_DEVELOPMENT_BUILD__) observeSessionWireReceived(event.payload);
    handler(event.payload);
  });
}

export function listenSettingsChanged(
  handler: (settings: SettingsSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<SettingsSnapshot>("settings-changed", (event) =>
    handler(event.payload),
  );
}

export function listenOverlayControlMode(
  handler: (mode: OverlayControlMode) => void,
): Promise<UnlistenFn> {
  return listen<OverlayControlMode>("overlay-control-mode", (event) =>
    handler(event.payload),
  );
}

/** View-local logical coordinates for a nonactivating macOS overlay. */
export type OverlayPointerMotion = { x: number; y: number } | null;

export function setOverlayPointerCursor(point: NonNullable<OverlayPointerMotion>, pointing: boolean): Promise<boolean> {
  return invoke<boolean>("overlay_set_pointer_cursor", { ...point, pointing });
}

export function listenOverlayPointerMotion(
  handler: (point: OverlayPointerMotion) => void,
): Promise<UnlistenFn> {
  return listen<OverlayPointerMotion>("overlay-pointer-motion", (event) =>
    handler(event.payload),
  );
}

/** Receives an explicit category intent when another app window opens the
 * settings window for a specific task. */
export function listenSettingsNavigation(
  handler: (target: SettingsNavigationTarget) => void,
): Promise<UnlistenFn> {
  return listen<SettingsNavigationTarget>("settings-navigate", (event) =>
    handler(event.payload),
  );
}

/** Completes the navigation handshake after SettingsView has installed its
 * listener. This matters only when the native window had to be recreated. */
export function announceSettingsNavigationReady(): Promise<void> {
  return emit("settings-navigation-ready");
}

export function sessionArchiveState(): Promise<SessionArchiveState> {
  return invoke("session_archive_state");
}

export function sessionTranscriptPage(query: string, page: number): Promise<TranscriptPage> {
  return invoke("session_transcript_page", { query, page });
}

export function sessionHistoryList(): Promise<SessionHistoryItem[]> {
  return invoke("session_history_list");
}

export function sessionHistoryPage(id: string, query: string, page: number): Promise<TranscriptPage> {
  return invoke("session_history_page", { id, query, page });
}

export function sessionHistoryAudio(id: string, audioSource?: AudioSource): Promise<ArrayBuffer> {
  return invoke("session_history_audio", { id, audioSource: audioSource ?? null });
}

export function sessionHistoryDelete(id: string): Promise<void> {
  return invoke("session_history_delete", { id });
}

export function sessionExport(kind: SessionExportKind, id?: string, audioSource?: AudioSource): Promise<boolean> {
  return invoke("session_export", { kind, id: id ?? null, audioSource: audioSource ?? null });
}

export function sessionArchiveClear(): Promise<void> {
  return invoke("session_archive_clear");
}

export interface DesktopShortcutCommands {
  toggleSession: string;
  toggleImmersive: string;
  cycleSubtitleDisplay: string;
}

export function appDesktopShortcutCommands(): Promise<DesktopShortcutCommands | null> {
  return invoke("app_desktop_shortcut_commands");
}

export interface ConnectionDiagnostic {
  credential: "present" | "missing" | "unavailable" | "localDevUnavailable" | "serviceUnavailable" | "accessDenied" | "invalid";
  service: "available" | "unavailable" | "notTested";
  reason: null | "credentialsMissing" | "credentialsUnavailable" | "localDevCredentialsUnavailable" | "credentialsServiceUnavailable" | "credentialsAccessDenied" | "invalidConfiguration" | "authenticationRejected" | "serviceRejected" | "timeout" | "unreachable" | "textTranslationNotConfigured";
  elapsedMs?: number | null;
}
export type ConnectionCheckStage = "speech" | "text";
export function testProfileConnection(profileId: string, stage?: ConnectionCheckStage): Promise<ConnectionDiagnostic> {
  return invoke("profile_test_connection", stage ? { profileId, stage } : { profileId });
}
