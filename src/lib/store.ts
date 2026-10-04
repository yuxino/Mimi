import { audio3ErrorMessage } from "./audio3Errors";
import { audioSourceErrorMessage } from "./windowsAudioSource";
import { applicationAudioError } from "./applicationAudio";
import { audioInputErrorMessage } from "./audioInput";
import { credentialErrorMessage } from "./connectionDiagnostics";
import { shareUnchangedSubtitleHistory } from "./sessionSnapshot";
import { observeSessionStoreApplied } from "./developmentTrace";
import { DEFAULT_NETWORK_PROXY, validateNetworkProxy } from "./networkProxy";
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
  overlayShow,
  profileCreate,
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
  trayPanelHide,
  type SettingsNavigationTarget,
} from "./ipc";
import { setStoredUiLanguage } from "./i18n";
import {
  capabilitiesForProvider,
  capabilitiesForProfile,
  isChatCompletionsTranslation,
  isCustomSpeechProvider,
  effectiveProviderForProfile,
  textTranslationForProfile,
  sourceLanguagesForSettings,
  targetLanguagesForSettings,
  targetLanguageAfterSourceSwitch,
  translationModesForSettings,
} from "./providerCapabilities";
import {
  SnapshotStreamBootstrap,
  mergeSettingsSnapshot,
  SettingsSaveCoordinator,
  SnapshotBootstrapTimeoutError,
  SnapshotResponseGate,
} from "./settingsState";
import type {
  ProfileNetworkProxyDraft,
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
  profiles: [
    {
      id: "alibaba-default",
      name: "Alibaba Cloud",
      provider: "alibabaCloud",
      credentialState: isTauri ? "unavailable" : "present",
    },
  ],
  activeProfileId: "alibaba-default",
  sourceLanguage: "auto",
  targetLanguage: "zh",
  translationMode: "turbo",
  fontSize: 16,
  subtitleBackgroundOpacity: 80,
  subtitleColor: "white",
  microphoneSubtitleColor: "yellow",
  subtitleAlignment: "center",
  subtitleDisplayMode: "translation",
  showIntermediateSubtitles: true,
  showSubtitleDividers: false,
  showSubtitleTimestamps: false,
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
  updateProfile: (
    profileId: string,
    name: string,
    proxies?: ProfileNetworkProxyDraft,
  ) => Promise<SettingsSnapshot>;
  selectProfile: (profileId: string) => Promise<SettingsSnapshot>;
  deleteProfile: (profileId: string) => Promise<SettingsSnapshot>;
  saveProfileCredentials: (
    profileId: string,
    credentials: ProviderCredentialsInput,
  ) => Promise<SettingsSnapshot>;
  deleteProfileAPIKey: (profileId: string) => Promise<SettingsSnapshot>;
  setOverlayCollapsed: (collapsed: boolean) => Promise<void>;
  setOverlayLocked: (locked: boolean) => Promise<void>;
  showOverlay: () => Promise<void>;
  hideTrayPanel: () => Promise<void>;
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
    ? credentialErrorMessage(state.session.status.message) ?? applicationAudioError(state.session.status.message) ?? audioInputErrorMessage(state.session.status.message) ?? audioSourceErrorMessage(state.session.status.message) ?? audio3ErrorMessage(state.session.status.message) ?? state.session.status.message
    : null;
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
            settingsSaveCoordinator.invalidate();
            set({ settings, hasSettingsSnapshot: true });
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
    const attempt = bootstrap.initialize().then(() => {
      if (bootstrap !== snapshotBootstrap || !bootstrap.ready) return;
      set({ initialized: true, initializationStatus: "ready", initializationError: null });
    }).catch((error: unknown) => {
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
    if (
      sessionSettingsAreChanging(current.session) ||
      !sourceLanguagesForSettings(current.settings).includes(language)
    ) {
      return;
    }
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
    if (!targets.includes("original") || !targets.includes(language)) throw new Error("target_switch_unsupported");
    if (isTauri) {
      await sessionSwitchTargetLanguage(language);
      return;
    }
    const settings = { ...current.settings, targetLanguage: language, languageCapabilities: undefined };
    const sources = sourceLanguagesForSettings(settings);
    if (!sources.includes(settings.sourceLanguage)) settings.sourceLanguage = sources[0]!;
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
      set({ settings: mergeSettingsSnapshot(previous, draft) });
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
        return snapshot;
      }
      return get().settings;
    }
    const current = get().settings;
    if (current.profiles.filter(profile => profile.credentialStorage !== "localDevFile").length >= 20) throw new Error("profile-limit");
    const id = `mock-${provider}-${Date.now()}`;
    const snapshot: SettingsSnapshot = {
      ...current,
      profiles: [
        ...current.profiles,
        { id, name, provider, credentialState: "missing", ...(isCustomSpeechProvider(provider) ? { speechCredentialState: "missing" as const, textCredentialState: "missing" as const } : {}) },
      ],
    };
    set({ settings: snapshot });
    return snapshot;
  },

  updateProfile: async (profileId, name, proxies) => {
    ensureProfileMutationsAllowed(get().session);
    if (isTauri) {
      const revision = settingsResponseGate.capture();
      const snapshot = await profileUpdate(profileId, name, proxies);
      if (settingsResponseGate.applyIfCurrent(revision)) {
        settingsSaveCoordinator.invalidate();
        set({ settings: snapshot });
        return snapshot;
      }
      return get().settings;
    }
    const current = get().settings;
    const snapshot: SettingsSnapshot = {
      ...current,
      profiles: current.profiles.map((profile) =>
        profile.id === profileId ? { ...profile, name, ...proxies } : profile,
      ),
    };
    set({ settings: snapshot });
    return snapshot;
  },

  selectProfile: async (profileId) => {
    ensureProfileMutationsAllowed(get().session);
    if (isTauri) {
      const revision = settingsResponseGate.capture();
      const snapshot = await profileSelect(profileId);
      if (settingsResponseGate.applyIfCurrent(revision)) {
        settingsSaveCoordinator.invalidate();
        set({ settings: snapshot });
        return snapshot;
      }
      return get().settings;
    }
    const current = get().settings;
    const selected = current.profiles.find((profile) => profile.id === profileId);
    if (!selected) throw new Error("profile-not-found");
    const snapshot = settingsAfterMockProfileSelection(current, effectiveProviderForProfile(selected));
    if (isCustomSpeechProvider(selected.provider)) {
      const capabilities = capabilitiesForProfile(selected, current.targetLanguage);
      snapshot.sourceLanguage = capabilities.sourceLanguages.includes(current.sourceLanguage) ? current.sourceLanguage : capabilities.sourceLanguages[0]!;
      snapshot.targetLanguage = capabilities.targetLanguages.includes(current.targetLanguage) ? current.targetLanguage : capabilities.targetLanguages[0]!;
    }
    snapshot.activeProfileId = profileId;
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
    const current = get().settings;
    if (current.profiles.length <= 1) throw new Error("last-profile");
    const profiles = current.profiles.filter((profile) => profile.id !== profileId);
    const activeProfileId =
      current.activeProfileId === profileId
        ? (profiles[0]?.id ?? "")
        : current.activeProfileId;
    const snapshot: SettingsSnapshot = { ...current, profiles, activeProfileId };
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
    const current = get().settings;
    if (credentials.kind === "alibabaTranslation") {
      const profile = current.profiles.find((profile) => profile.id === profileId);
      if (!profile || (!isCustomSpeechProvider(profile.provider) && !["alibabaCloud", "deepLX"].includes(profile.provider))) throw new Error("provider-mismatch");
      if (isCustomSpeechProvider(profile.provider) ? credentials.apiKey.trim() : !credentials.apiKey.trim() && profile.credentialState !== "present") throw new Error("credential-empty");
      if (credentials.textTranslation === "deepLX" && !credentials.endpoint.trim() && textTranslationForProfile(profile) !== "deepLX") throw new Error("credential-empty");
      if (credentials.textTranslation === "deepL" && !credentials.token.trim() && textTranslationForProfile(profile) !== "deepL") throw new Error("credential-empty");
      if (isChatCompletionsTranslation(credentials.textTranslation) && textTranslationForProfile(profile) !== credentials.textTranslation && (!credentials.endpoint.trim() || !credentials.model.trim())) throw new Error("credential-empty");
    } else if (credentials.kind === "customSpeech") {
      const profile = current.profiles.find(profile => profile.id === profileId);
      if (!profile || !isCustomSpeechProvider(profile.provider)) throw new Error("provider-mismatch");
      if ((profile.speechCredentialState !== "present" && (!credentials.endpoint.trim() || !credentials.model.trim() || !credentials.apiKey.trim())) || (credentials.endpoint.trim() && !credentials.apiKey.trim())) throw new Error("credential-empty");
    } else if (Object.entries(credentials).some(([field, value]) => field !== "kind" && field !== "token" && !value.trim())) {
      throw new Error("credential-empty");
    }
    const snapshot: SettingsSnapshot = {
      ...current,
      profiles: current.profiles.map((profile) =>
        profile.id === profileId
          ? { ...profile, credentialState: "present", ...(credentials.kind === "alibabaTranslation" ? { textTranslation: credentials.textTranslation, ...(isCustomSpeechProvider(profile.provider) ? { textCredentialState: "present" as const, credentialState: profile.speechCredentialState ?? "missing" } : {}) } : {}), ...(credentials.kind === "customSpeech" ? { speechCredentialState: "present" as const, credentialState: textTranslationForProfile(profile) === "followService" || profile.textCredentialState === "present" ? "present" as const : "missing" as const } : {}) }
          : profile,
      ),
    };
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
    const current = get().settings;
    const snapshot: SettingsSnapshot = {
      ...current,
      profiles: current.profiles.map((profile) =>
        profile.id === profileId
          ? { ...profile, credentialState: "missing", ...(isCustomSpeechProvider(profile.provider) ? { speechCredentialState: "missing" as const, textCredentialState: "missing" as const } : {}) }
          : profile,
      ),
    };
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

  showOverlay: async () => {
    if (isTauri) await overlayShow();
  },

  hideTrayPanel: async () => {
    if (isTauri) await trayPanelHide();
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

function settingsAfterMockProfileSelection(
  current: SettingsSnapshot,
  provider: ServiceProvider,
): SettingsSnapshot {
  if (provider === "alibabaCloud") return { ...current };
  const capabilities = capabilitiesForProvider(provider);
  const sourceLanguage =
    capabilities.sourceLanguages.find(
      (source) =>
        !(
          (source === "zh" && current.targetLanguage === "zh") ||
          (source === "en" && current.targetLanguage === "en") ||
          (source === "ja" && current.targetLanguage === "ja")
        ),
    ) ?? capabilities.sourceLanguages[0];
  return {
    ...current,
    sourceLanguage: sourceLanguage ?? current.sourceLanguage,
    targetLanguage:
      current.targetLanguage === "original" ? "zh" : current.targetLanguage,
    translationMode: "turbo",
  };
}

/** Settings events update each window in place, preserving updater resources. */
function syncUiLanguageFromSettings(settings: SettingsSnapshot): void {
  setStoredUiLanguage(settings.uiLanguage ?? "system");
}
