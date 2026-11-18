import { createPreviewProfile, duplicatePreviewProfile, updatePreviewProfile, selectPreviewProfile, deletePreviewProfile, savePreviewCredentials, deletePreviewCredentials } from "./profiles/browserProfilePreview";
import { audio3ErrorMessage, audio3ErrorSummary } from "./providers/audio3Errors";
import { localizedSessionErrorSummary } from "./session/sessionErrorPresentation";
import { audioSourceErrorMessage } from "./audio/windowsAudioSource";
import { applicationAudioError } from "./audio/applicationAudio";
import { audioInputErrorMessage } from "./audio/audioInput";
import { credentialErrorMessage } from "./providers/connectionDiagnostics";
import { shareUnchangedSubtitleHistory } from "./session/sessionSnapshot";
import { observeSessionStoreApplied } from "./development/developmentTrace";
import { DEFAULT_NETWORK_PROXY, validateNetworkProxy } from "./providers/networkProxy";
/**
 * Global zustand store. In Tauri it forwards every action to the Rust backend
 * and applies `session-state` / `settings-changed` events as they arrive. In a
 * plain `vite dev` session (no `__TAURI_INTERNALS__`) it emulates the backend
 * locally so the UI can be developed without the Rust side running.
 */

import { create } from "zustand";
import {
  appQuit,
  appShowSettings,
  isTauri,
  listenSessionState,
  listenSettingsChanged,
  overlaySetCollapsed,
  overlaySetLocked,
  profileCreate,
  profileDuplicate,
  profileDelete,
  profileDeleteAPIKey,
  profileSaveCredentials,
  profileSelect,
  profileUpdate,
  sessionClearSubtitles,
  sessionGetState,
  sessionStart,
  sessionStop,
  sessionSwitchSourceLanguage,
  sessionSwitchTargetLanguage,
  sessionSwitchAudioInput,
  sessionSwitchSystemAudioTarget,
  sessionSwitchTranslationMode,
  sessionTogglePaused,
  settingsGet,
  settingsSave,
  type SettingsNavigationTarget,
} from "./ipc";
import { I18N, setStoredUiLanguage } from "./i18n";
import {
  sourceLanguagesForSettings,
  targetLanguagesForSettings,
  targetLanguageAfterSourceSwitch,
  translationModesForSettings,
} from "./providers/providerCapabilities";
import {
  SnapshotStreamBootstrap,
  mergeSettingsSnapshot,
  SettingsSaveCoordinator,
  SnapshotBootstrapTimeoutError,
  SnapshotResponseGate,
} from "./settings/settingsState";
import type {
  ProfileOptionsDraft,
  AudioInput,
  ProviderCredentialsInput,
  SessionStateEvent,
  SettingsDraft,
  SettingsSnapshot,
  ServiceProvider,
  SourceLanguage,
  SystemAudioTarget,
  TargetLanguage,
  SubtitleSnapshot,
  TranslationMode,
} from "./types";

const EMPTY_SUBTITLES: SubtitleSnapshot = {
  source: { text: "", isFinal: false },
  translation: { text: "", isFinal: false },
  history: [],
};

const INITIAL_SESSION: SessionStateEvent = {
  apiLatencyMs: null,
  translationLatencyMs: null,
  translationLatencyKind: null,
  status: { kind: "idle" },
  isActive: false,
  isPaused: false,
  isOverlayCollapsed: false,
  subtitles: EMPTY_SUBTITLES,
  detectedLanguage: null,
  isTranslationPending: false,
  isTranslationTimedOut: false,
};

const INITIAL_SETTINGS: SettingsSnapshot = {
  // The browser-only preview has an explicit demo profile. Native startup waits
  // for the real catalog and never suggests a provider before the user adds one.
  profiles: isTauri ? [] : [
    {
      id: "alibaba-default",
      name: "Alibaba Cloud",
      provider: "alibabaCloud",
      credentialState: "present",
    },
  ],
  activeProfileId: isTauri ? "" : "alibaba-default",
  sourceLanguage: "auto",
  targetLanguage: "zh",
  translationMode: "turbo",
  fontSize: 16,
  subtitleFontFamily: "",
  subtitleBackgroundOpacity: 80,
  subtitleColor: "white",
  microphoneSubtitleColor: "yellow",
  subtitleAlignment: "center",
  subtitleDisplayMode: "translation",
  showIntermediateSubtitles: true,
  showSubtitleDividers: false,
  showSubtitleTimestamps: false,
  customSubtitleCountEnabled: false,
  visibleSubtitleCount: 3,
  pulseAnimation: null,
  pulseStyle: "ribbon",
  subtitleAnimation: null,
  subtitleBlendsWithBackground: false,
  isOverlayLocked: false,
  uiLanguage: null,
  retainSessionHistory: false,
  recordSessionAudio: false,
  audioInput: "system",
  microphoneInputAvailable: false,
  windowsAudioSource: "",
  systemAudioTarget: { kind: "system" },
  showInDock: true,
  networkProxy: DEFAULT_NETWORK_PROXY,
};

interface StoreState {
  session: SessionStateEvent;
  settings: SettingsSnapshot;
  initialized: boolean;
  initializationStatus: "idle" | "loading" | "ready" | "error";
  initializationError: "timeout" | "unavailable" | null;
  hasSettingsSnapshot: boolean;
  init: () => Promise<void>;
  start: () => Promise<void>;
  stop: () => Promise<void>;
  togglePaused: () => Promise<void>;
  clearSubtitles: () => Promise<void>;
  switchSourceLanguage: (language: SourceLanguage) => Promise<void>;
  switchTargetLanguage: (language: TargetLanguage) => Promise<void>;
  switchAudioInput: (input: AudioInput) => Promise<void>;
  switchSystemAudioTarget: (target: SystemAudioTarget) => Promise<void>;
  switchTranslationMode: (mode: TranslationMode) => Promise<void>;
  saveSettings: (draft: SettingsDraft) => Promise<void>;
  createProfile: (
    provider: ServiceProvider,
    name: string,
  ) => Promise<SettingsSnapshot>;
  duplicateProfile: (profileId: string, name: string) => Promise<SettingsSnapshot>;
  updateProfile: (
    profileId: string,
    name: string | undefined,
    options?: ProfileOptionsDraft,
  ) => Promise<SettingsSnapshot>;
  selectProfile: (profileId: string, sourceLanguage?: SourceLanguage) => Promise<SettingsSnapshot>;
  deleteProfile: (profileId: string) => Promise<SettingsSnapshot>;
  saveProfileCredentials: (
    profileId: string,
    credentials: ProviderCredentialsInput,
  ) => Promise<SettingsSnapshot>;
  deleteProfileAPIKey: (profileId: string) => Promise<SettingsSnapshot>;
  setOverlayCollapsed: (collapsed: boolean) => Promise<void>;
  setOverlayLocked: (locked: boolean) => Promise<void>;
  showSettings: (target?: SettingsNavigationTarget) => Promise<void>;
  quit: () => Promise<void>;
}

type SessionStoreSlice = Pick<StoreState, "session">;

/**
 * Stable primitive selectors for native windows that do not render subtitle
 * text. Provider drafts replace the session snapshot many times per second;
 * returning the status object would repaint hidden settings/tray windows on
 * every replacement even when their visible state did not change.
 */
export function selectSessionStatusKind(state: SessionStoreSlice) {
  return state.session.status.kind;
}

export function selectSessionErrorMessage(state: SessionStoreSlice) {
  return state.session.status.kind === "error"
    ? credentialErrorMessage(state.session.status.message) ?? applicationAudioError(state.session.status.message) ?? audioInputErrorMessage(state.session.status.message) ?? audioSourceErrorMessage(state.session.status.message) ?? audio3ErrorMessage(state.session.status.message) ?? I18N.settings.sessionError
    : null;
}

export function selectSessionErrorSummary(state: SessionStoreSlice) {
  const message = selectSessionErrorMessage(state);
  if (message === null || state.session.status.kind !== "error") return null;
  return audio3ErrorSummary(state.session.status.message) ?? localizedSessionErrorSummary(message);
}

export function selectHasRecognizingSourceDraft(state: SessionStoreSlice) {
  const source = state.session.subtitles.source;
  return (source.text !== "" && !source.isFinal) ||
    (state.session.subtitles.tracks?.some(track => track.source.text !== "" && !track.source.isFinal) ?? false);
}

const settingsSaveCoordinator = new SettingsSaveCoordinator();
const settingsResponseGate = new SnapshotResponseGate();
let snapshotBootstrap: SnapshotStreamBootstrap<SettingsSnapshot, SessionStateEvent> | null = null;
let initializationAttempt: Promise<void> | null = null;
let initializationGeneration = 0;
let overlayCollapseRequest = 0;

/** A timeout is recoverable; only an actual WebView teardown expires streams. */
export function disposeStoreSnapshotStreams(): void {
  initializationGeneration += 1;
  snapshotBootstrap?.dispose();
  snapshotBootstrap = null;
  initializationAttempt = null;
}

if (isTauri && typeof window !== "undefined") {
  window.addEventListener("pagehide", (event) => {
    if (!event.persisted) disposeStoreSnapshotStreams();
  });
}

export const useStore = create<StoreState>()((set, get) => ({
  session: INITIAL_SESSION,
  settings: INITIAL_SETTINGS,
  initialized: false,
  initializationStatus: isTauri ? "idle" : "ready",
  initializationError: null,
  hasSettingsSnapshot: !isTauri,

  init: () => {
    if (get().initialized) return Promise.resolve();
    if (initializationAttempt) return initializationAttempt;
    // Set the guard synchronously: React StrictMode double-invokes effects
    // in development, and both calls would otherwise register listeners.
    if (!isTauri) {
      set({ initialized: true, initializationStatus: "ready", hasSettingsSnapshot: true });
      return Promise.resolve();
    }
    set({ initializationStatus: "loading", initializationError: null });
    if (snapshotBootstrap === null) {
      const generation = ++initializationGeneration;
      snapshotBootstrap = new SnapshotStreamBootstrap(
        {
          listenSettings: listenSettingsChanged,
          listenSession: listenSessionState,
          getSettings: settingsGet,
          getSession: sessionGetState,
        },
        {
          applySettings: (settings) => {
            if (generation !== initializationGeneration) return;
            settingsResponseGate.advance();
            set({ settings: settingsSaveCoordinator.receiveSnapshot(settings), hasSettingsSnapshot: true });
            // Language switches initiated from any window reach every other
            // window through this event without reloading the WebView.
            syncUiLanguageFromSettings(settings);
          },
          applySession: (session) => {
            if (generation !== initializationGeneration) return;
            set((state) => ({ session: shareUnchangedSubtitleHistory(state.session, session) }));
            if (__MIMI_DEVELOPMENT_BUILD__) observeSessionStoreApplied(get().session);
          },
          onReady: () => {
            if (generation !== initializationGeneration) return;
            set({ initialized: true, initializationStatus: "ready", initializationError: null });
          },
        },
      );
    }
    const bootstrap = snapshotBootstrap;
    // onReady owns successful hydration, including events arriving after a deadline.
    const attempt = bootstrap.initialize().catch((error: unknown) => {
      if (bootstrap !== snapshotBootstrap || bootstrap.ready) return;
      // Preserve listeners and their generation. Retrying reuses any pending
      // OS read; later settings events/late success can recover every window.
      set({ initialized: false, initializationStatus: "error", initializationError: error instanceof SnapshotBootstrapTimeoutError ? "timeout" : "unavailable" });
    }).finally(() => {
      if (initializationAttempt === attempt) initializationAttempt = null;
    });
    initializationAttempt = attempt;
    return attempt;
  },

  start: async () => {
    if (isTauri) {
      await sessionStart();
      return;
    }
    // Mock: flip into listening and seed a couple of Japanese→Chinese lines.
    const now = Date.now();
    set((state) => ({
      session: {
        apiLatencyMs: null,
        translationLatencyMs: null,
        translationLatencyKind: null,
        status: { kind: "listening" },
        isActive: true,
        isPaused: false,
        isOverlayCollapsed: state.session.isOverlayCollapsed,
        subtitles: {
          source: { text: "今日は映画について話しましょう。", isFinal: true },
          translation: { text: "今天咱们聊聊电影吧。", isFinal: true },
          history: [
            {
              source: "今日は映画について話しましょう。",
              translation: "今天咱们聊聊电影吧。",
              createdAt: now - 6000,
            },
            {
              source: "主人公は駅で友達を待っています。",
              translation: "主人公正在车站等朋友呢。",
              createdAt: now - 3000,
            },
          ],
        },
        detectedLanguage: "ja",
        isTranslationPending: false,
        isTranslationTimedOut: false,
      },
    }));
  },

  stop: async () => {
    if (isTauri) {
      await sessionStop();
      return;
    }
    set((state) => ({
      session: {
        ...state.session,
        status: { kind: "idle" },
        apiLatencyMs: null,
        translationLatencyMs: null,
        translationLatencyKind: null,
        isActive: false,
        isPaused: false,
        isTranslationPending: false,
        isTranslationTimedOut: false,
      },
    }));
  },

  togglePaused: async () => {
    if (isTauri) {
      await sessionTogglePaused();
      return;
    }
    set((state) => ({
      session: { ...state.session, isPaused: !state.session.isPaused },
    }));
  },

  clearSubtitles: async () => {
    if (isTauri) {
      await sessionClearSubtitles();
      return;
    }
    set((state) => ({
      session: {
        ...state.session,
        subtitles: EMPTY_SUBTITLES,
        isTranslationPending: false,
        isTranslationPreviewPending: false,
        isTranslationTimedOut: false,
        translationRecovery: null,
        translationLatencyMs: null,
        translationLatencyKind: null,
      },
    }));
  },

  switchSourceLanguage: async (language) => {
    const current = get();
    if (sessionSettingsAreChanging(current.session)) throw new Error("source_switch_busy");
    if (!sourceLanguagesForSettings(current.settings).includes(language)) throw new Error("source_switch_unsupported");
    if (isTauri) {
      await sessionSwitchSourceLanguage(language);
      return;
    }
    set((state) => ({
      settings: {
        ...state.settings,
        sourceLanguage: language,
        targetLanguage: targetLanguageAfterSourceSwitch(
          state.settings,
          language,
        ),
      },
    }));
  },

  switchTargetLanguage: async (language) => {
    const current = get();
    if (sessionSettingsAreChanging(current.session)) throw new Error("target_switch_busy");
    const targets = targetLanguagesForSettings(current.settings);
    if (!targets.includes(language)) throw new Error("target_switch_unsupported");
    if (isTauri) {
      await sessionSwitchTargetLanguage(language);
      return;
    }
    const settings = { ...current.settings, targetLanguage: language, languageCapabilities: undefined };
    const sources = sourceLanguagesForSettings(settings);
    if (!sources.includes(settings.sourceLanguage)) settings.sourceLanguage = sources[0] ?? settings.sourceLanguage;
    set({ settings });
  },

  switchTranslationMode: async (mode) => {
    const current = get();
    if (
      sessionSettingsAreChanging(current.session) ||
      !translationModesForSettings(current.settings).includes(mode)
    ) {
      return;
    }
    if (isTauri) {
      await sessionSwitchTranslationMode(mode);
      return;
    }
    set((state) => ({
      settings: { ...state.settings, translationMode: mode },
    }));
  },

  switchAudioInput: async (input) => {
    const current = get();
    if (input === (current.settings.audioInput ?? "system")) return;
    if (sessionSettingsAreChanging(current.session)) throw new Error("audio_input_switch_busy");
    if (isTauri) {
      await sessionSwitchAudioInput(input);
      return;
    }
    // The browser preview preserves the live/paused state and confirmed text,
    // just as the native reconfiguration path does. It never opens devices.
    set(state => ({ settings: mergeSettingsSnapshot(state.settings, { audioInput: input }) }));
  },

  switchSystemAudioTarget: async (target) => {
    const current = get();
    if (JSON.stringify(target) === JSON.stringify(current.settings.systemAudioTarget ?? { kind: "system" })) return;
    if (sessionSettingsAreChanging(current.session)) throw new Error("audio_input_switch_busy");
    if (isTauri) {
      await sessionSwitchSystemAudioTarget(target);
      return;
    }
    set(state => ({ settings: mergeSettingsSnapshot(state.settings, { systemAudioTarget: target }) }));
  },

  saveSettings: async (draft) => {
    const previous = get().settings;
    if ((draft.systemAudioTarget !== undefined || (draft.audioInput !== undefined && draft.audioInput !== (previous.audioInput ?? "system"))) &&
      (get().session.isActive || get().session.isPaused || sessionSettingsAreChanging(get().session))) {
      throw new Error("audio_input_change_requires_stop");
    }
    if (!isTauri) {
      if (draft.networkProxy !== undefined && (get().session.isActive || get().session.isPaused || sessionSettingsAreChanging(get().session))) {
        throw new Error("network_proxy_change_requires_stop");
      }
      if (draft.networkProxy !== undefined) {
        const validated = validateNetworkProxy(draft.networkProxy.mode, draft.networkProxy.url);
        if ("error" in validated) {
          const label = { invalidUrl: "network_proxy_invalid_url", unsupportedScheme: "network_proxy_unsupported_scheme", authenticationUnsupported: "network_proxy_authentication_unsupported" }[validated.error];
          throw new Error(label);
        }
      }
      const settings = mergeSettingsSnapshot(previous, draft);
      if (draft.targetLanguage !== undefined) {
        const sources = sourceLanguagesForSettings(settings);
        if (!sources.includes(settings.sourceLanguage)) settings.sourceLanguage = sources[0] ?? settings.sourceLanguage;
      }
      set({ settings });
      return;
    }
    await settingsSaveCoordinator.save(previous, draft, settingsSave, (settings) =>
      {
        settingsResponseGate.advance();
        set({ settings });
      },
    );
  },

  createProfile: async (provider, name) => {
    ensureProfileMutationsAllowed(get().session);
    if (isTauri) {
      const revision = settingsResponseGate.capture();
      const snapshot = await profileCreate(provider, name);
      if (settingsResponseGate.applyIfCurrent(revision)) {
        settingsSaveCoordinator.invalidate();
        set({ settings: snapshot });
      }
      return snapshot;
    }
    const snapshot = createPreviewProfile(get().settings, provider, name);
    set({ settings: snapshot });
    return snapshot;
  },

  duplicateProfile: async (profileId, name) => {
    ensureProfileMutationsAllowed(get().session);
    if (isTauri) {
      const revision = settingsResponseGate.capture();
      const snapshot = await profileDuplicate(profileId, name);
      if (settingsResponseGate.applyIfCurrent(revision)) {
        settingsSaveCoordinator.invalidate();
        set({ settings: snapshot });
      }
      return snapshot;
    }
    const snapshot = duplicatePreviewProfile(get().settings, profileId, name);
    set({ settings: snapshot });
    return snapshot;
  },

  updateProfile: async (profileId, name, options) => {
    ensureProfileMutationsAllowed(get().session);
    if (isTauri) {
      const revision = settingsResponseGate.capture();
      const snapshot = await profileUpdate(profileId, name, options);
      if (settingsResponseGate.applyIfCurrent(revision)) {
        settingsSaveCoordinator.invalidate();
        set({ settings: snapshot });
      }
      // This response acknowledges the requested metadata even if a newer
      // event already owns global state. Returning that unrelated event could
      // make an autosave editor mistake a successful rename for a rejection.
      return snapshot;
    }
    const snapshot = updatePreviewProfile(get().settings, profileId, name, options);
    set({ settings: snapshot });
    return snapshot;
  },

  selectProfile: async (profileId, sourceLanguage) => {
    if (profileId === get().settings.activeProfileId && !sourceLanguage
      && !get().settings.profiles.find(profile => profile.id === profileId)?.languagePreset) return get().settings;
    if (sessionSettingsAreChanging(get().session)) throw new Error("profile_switch_busy");
    if (sourceLanguage) {
      ensureProfileMutationsAllowed(get().session);
      if (get().settings.profiles.find(profile => profile.id === profileId)?.provider !== "appleSpeech" || sourceLanguage === "auto") {
        throw new Error("apple_speech_language_unsupported");
      }
    }
    if (isTauri) {
      const revision = settingsResponseGate.capture();
      const snapshot = await profileSelect(profileId, sourceLanguage);
      if (settingsResponseGate.applyIfCurrent(revision)) {
        settingsSaveCoordinator.invalidate();
        set({ settings: snapshot });
        return snapshot;
      }
      return get().settings;
    }
    const snapshot = selectPreviewProfile(get().settings, profileId, sourceLanguage);
    set({ settings: snapshot });
    return snapshot;
  },

  deleteProfile: async (profileId) => {
    ensureProfileMutationsAllowed(get().session);
    if (isTauri) {
      const revision = settingsResponseGate.capture();
      const snapshot = await profileDelete(profileId);
      if (settingsResponseGate.applyIfCurrent(revision)) {
        settingsSaveCoordinator.invalidate();
        set({ settings: snapshot });
        return snapshot;
      }
      return get().settings;
    }
    const snapshot = deletePreviewProfile(get().settings, profileId);
    set({ settings: snapshot });
    return snapshot;
  },

  saveProfileCredentials: async (profileId, credentials) => {
    ensureProfileMutationsAllowed(get().session);
    if (isTauri) {
      const revision = settingsResponseGate.capture();
      const snapshot = await profileSaveCredentials(profileId, credentials);
      if (settingsResponseGate.applyIfCurrent(revision)) {
        settingsSaveCoordinator.invalidate();
        set({ settings: snapshot });
        return snapshot;
      }
      return get().settings;
    }
    const snapshot = savePreviewCredentials(get().settings, profileId, credentials);
    set({ settings: snapshot });
    return snapshot;
  },

  deleteProfileAPIKey: async (profileId) => {
    ensureProfileMutationsAllowed(get().session);
    if (isTauri) {
      const revision = settingsResponseGate.capture();
      const snapshot = await profileDeleteAPIKey(profileId);
      if (settingsResponseGate.applyIfCurrent(revision)) {
        settingsSaveCoordinator.invalidate();
        set({ settings: snapshot });
        return snapshot;
      }
      return get().settings;
    }
    const snapshot = deletePreviewCredentials(get().settings, profileId);
    set({ settings: snapshot });
    return snapshot;
  },

  setOverlayCollapsed: async (collapsed) => {
    // Optimistic local update so the overlay layout switches immediately;
    // the backend event confirms it afterwards. Only the latest failed request
    // can undo its own still-current optimistic value, preserving new content
    // and newer presentation requests/events.
    const request = ++overlayCollapseRequest;
    const previous = get().session.isOverlayCollapsed;
    set((state) => ({
      session: { ...state.session, isOverlayCollapsed: collapsed },
    }));
    if (isTauri) {
      try {
        await overlaySetCollapsed(collapsed);
      } catch (error) {
        if (request === overlayCollapseRequest && get().session.isOverlayCollapsed === collapsed) {
          set(state => ({ session: { ...state.session, isOverlayCollapsed: previous } }));
        }
        throw error;
      }
    }
  },

  setOverlayLocked: async (locked) => {
    if (isTauri) {
      await overlaySetLocked(locked);
      return;
    }
    set((state) => ({
      settings: { ...state.settings, isOverlayLocked: locked },
    }));
  },

  showSettings: async (target) => {
    if (isTauri) await appShowSettings(target);
  },

  quit: async () => {
    if (isTauri) await appQuit();
  },
}));

function ensureProfileMutationsAllowed(session: SessionStateEvent): void {
  if (session.isActive || session.isPaused || sessionSettingsAreChanging(session)) throw new Error("session-active");
}

function sessionSettingsAreChanging(session: SessionStateEvent): boolean {
  return (
    session.status.kind === "connecting" || session.status.kind === "stopping"
  );
}

/** Settings events update each window in place, preserving updater resources. */
function syncUiLanguageFromSettings(settings: SettingsSnapshot): void {
  setStoredUiLanguage(settings.uiLanguage ?? "system");
}
