import { describe, expect, it } from "vitest";
import { isTauri } from "./ipc";
import {
  selectHasRecognizingSourceDraft,
  selectSessionErrorMessage,
  selectSessionErrorSummary,
  selectSessionStatusKind,
  useStore,
} from "./store";

describe("local preview store", () => {
  it("keeps profile names, recognition and translation aliases, and proxies independent across repeated metadata patches", async () => {
    const original = useStore.getState();
    const profile = original.settings.profiles[0];
    const proxy = { mode: "direct" as const, url: null };
    try {
      await useStore.getState().updateProfile(profile.id, "  B 站 · @home / (测试) 😀  ");
      await useStore.getState().updateProfile(profile.id, undefined, {
        textTranslationName: { route: "openAICompatible", name: "  翻译 & 字幕 + [本地] 🐱  " },
      });
      await useStore.getState().updateProfile(profile.id, undefined, { speechRecognitionName: "  Whisper · 本地  " });
      await useStore.getState().updateProfile(profile.id, "B 站 2");
      await useStore.getState().updateProfile(profile.id, undefined, { textNetworkProxy: proxy });
      expect(useStore.getState().settings.profiles[0]).toMatchObject({
        name: "B 站 2", speechRecognitionName: "Whisper · 本地", textTranslationNames: { openAICompatible: "翻译 & 字幕 + [本地] 🐱" }, textNetworkProxy: proxy,
      });
      await useStore.getState().updateProfile(profile.id, undefined, { speechRecognitionName: "  " });
      expect(useStore.getState().settings.profiles[0]).toMatchObject({
        name: "B 站 2", textTranslationNames: { openAICompatible: "翻译 & 字幕 + [本地] 🐱" }, textNetworkProxy: proxy,
      });
      expect(useStore.getState().settings.profiles[0].speechRecognitionName).toBeUndefined();
      await useStore.getState().updateProfile(profile.id, undefined, {
        textTranslationName: { route: "openAICompatible", name: "  " },
      });
      expect(useStore.getState().settings.profiles[0]).toMatchObject({ name: "B 站 2", textTranslationNames: {}, textNetworkProxy: proxy });
    } finally { useStore.setState(original, true); }
  });

  it.each([false, true])("switches the preview application without starting or resuming capture, paused=%s", async isPaused => {
    const original = useStore.getState();
    const session = { ...original.session, status: { kind: "listening" as const }, isActive: !isPaused, isPaused,
      subtitles: { ...original.session.subtitles, history: [{ source: "Synthetic source", translation: "Synthetic translation", createdAt: 1 }] },
    };
    try {
      useStore.setState({ session, settings: { ...original.settings, audioInput: "microphone", systemAudioTarget: { kind: "system" }, recordSessionAudio: true } });
      await useStore.getState().switchSystemAudioTarget({ kind: "application", id: "example.player", name: "Player" });
      expect(useStore.getState().settings).toMatchObject({ audioInput: "microphone", systemAudioTarget: { kind: "application", id: "example.player" }, recordSessionAudio: false });
      expect(useStore.getState().session).toBe(session);
      useStore.setState({ session: { ...session, status: { kind: "connecting" } } });
      await expect(useStore.getState().switchSystemAudioTarget({ kind: "system" })).rejects.toThrow("audio_input_switch_busy");
      expect(useStore.getState().settings.systemAudioTarget.kind).toBe("application");
    } finally { useStore.setState(original, true); }
  });

  it.each([false, true])("switches the preview inputs without losing confirmed text or changing pause=%s", async isPaused => {
    const original = useStore.getState();
    const session = { ...original.session, status: { kind: "listening" as const }, isActive: true, isPaused,
      subtitles: { ...original.session.subtitles, history: [{ audioSource: "system" as const, source: "Synthetic source.", translation: "Synthetic translation.", createdAt: 1 }] },
    };
    try {
      useStore.setState({ session, settings: { ...original.settings, audioInput: "system", recordSessionAudio: true } });
      await useStore.getState().switchAudioInput("both");
      expect(useStore.getState().settings).toMatchObject({ audioInput: "both", recordSessionAudio: false });
      expect(useStore.getState().session).toBe(session);
      useStore.setState({ session: { ...session, status: { kind: "connecting" } } });
      await expect(useStore.getState().switchAudioInput("system")).rejects.toThrow("audio_input_switch_busy");
      expect(useStore.getState().settings.audioInput).toBe("both");
    } finally { useStore.setState(original, true); }
  });
  it("defaults to system audio, preserves explicit microphone selection, and requires stopping before a change", async () => {
    const original = useStore.getState();
    expect(original.settings.audioInput).toBe("system");
    try {
      await useStore.getState().saveSettings({ audioInput: "microphone" });
      await useStore.getState().saveSettings({ fontSize: 19 });
      expect(useStore.getState().settings.audioInput).toBe("microphone");
      for (const state of [
        { status: { kind: "listening" as const }, isActive: true, isPaused: false },
        { status: { kind: "listening" as const }, isActive: false, isPaused: true },
        { status: { kind: "connecting" as const }, isActive: false, isPaused: false },
        { status: { kind: "stopping" as const }, isActive: false, isPaused: false },
      ]) {
        useStore.setState({ session: { ...original.session, ...state } });
        await expect(useStore.getState().saveSettings({ audioInput: "system" })).rejects.toThrow("audio_input_change_requires_stop");
        expect(useStore.getState().settings.audioInput).toBe("microphone");
      }
    } finally { useStore.setState({ settings: original.settings, session: original.session }); }
  });
  it("defaults to showing in the Dock and preserves an explicit hidden choice on unrelated saves", async () => {
    const original = useStore.getState().settings;
    expect(original.showInDock).toBe(true);
    try {
      await useStore.getState().saveSettings({ showInDock: false });
      await useStore.getState().saveSettings({ fontSize: 19 });
      expect(useStore.getState().settings.showInDock).toBe(false);
    } finally {
      useStore.setState({ settings: original });
    }
  });

  it("clears captions and pending translation without stopping the local preview", async () => {
    const original = useStore.getState();
    try {
      useStore.setState({ session: {
        ...original.session,
        status: { kind: "listening" },
        isActive: true,
        isTranslationPending: true,
        isTranslationPreviewPending: true,
        isTranslationTimedOut: true,
        translationLatencyMs: 350,
        translationLatencyKind: "request",
        translationRecovery: { reason: "rateLimited", retryAfterMs: 4_000, retryScheduled: true },
        subtitles: { ...original.session.subtitles, source: { text: "synthetic clear fixture", isFinal: false } },
      } });
      await useStore.getState().clearSubtitles();
      const cleared = useStore.getState().session;
      expect(cleared.status.kind).toBe("listening");
      expect(cleared.isActive).toBe(true);
      expect(cleared.subtitles.source.text).toBe("");
      expect(cleared.isTranslationPending).toBe(false);
      expect(cleared.isTranslationPreviewPending).toBe(false);
      expect(cleared.isTranslationTimedOut).toBe(false);
      expect(cleared.translationRecovery).toBeNull();
      expect(cleared.translationLatencyMs).toBeNull();
    } finally { useStore.setState({ session: original.session }); }
  });

  it("defaults proxies to system, rejects authenticated routes and blocks changes while paused", async () => {
    const original = useStore.getState();
    expect(original.settings.networkProxy).toEqual({ mode: "system", url: null });
    try {
      await expect(useStore.getState().saveSettings({ networkProxy: { mode: "custom", url: "http://user:synthetic-secret@127.0.0.1" } })).rejects.toThrow("network_proxy_authentication_unsupported");
      expect(useStore.getState().settings).toBe(original.settings);
      await useStore.getState().saveSettings({ networkProxy: { mode: "custom", url: "socks5h://127.0.0.1" } });
      expect(useStore.getState().settings.networkProxy).toEqual({ mode: "custom", url: "socks5h://127.0.0.1:1080" });
      useStore.setState({ session: { ...original.session, status: { kind: "listening" }, isPaused: true, isActive: false } });
      await expect(useStore.getState().saveSettings({ networkProxy: { mode: "direct", url: null } })).rejects.toThrow("network_proxy_change_requires_stop");
      expect(useStore.getState().settings.networkProxy.mode).toBe("custom");
    } finally { useStore.setState({ settings: original.settings, session: original.session }); }
  });
  it("keeps the synthetic provider ready outside Tauri", () => {
    expect(isTauri).toBe(false);
    expect(
      useStore.getState().settings.profiles[0]?.credentialState,
    ).toBe("present");
  });

  it("defaults sentence dividers off and saves the preview choice without changing the pipeline", async () => {
    const original = useStore.getState().settings;
    expect(original.showSubtitleDividers).toBe(false);
    try {
      await useStore.getState().saveSettings({ showSubtitleDividers: true });
      expect(useStore.getState().settings.showSubtitleDividers).toBe(true);
      expect(useStore.getState().settings.translationMode).toBe(original.translationMode);
      await useStore.getState().saveSettings({ fontSize: 19 });
      expect(useStore.getState().settings.showSubtitleDividers).toBe(true);
      await useStore.getState().saveSettings({ showSubtitleDividers: false });
      expect(useStore.getState().settings.showSubtitleDividers).toBe(false);
    } finally {
      useStore.setState({ settings: original });
    }
  });

  it("keeps subtitle churn out of native-window session selectors", () => {
    const current = useStore.getState();
    const first = {
      ...current,
      session: {
        ...current.session,
        status: { kind: "listening" as const },
        subtitles: {
          ...current.session.subtitles,
          source: { text: "draft one", isFinal: false },
        },
      },
    };
    const replacement = {
      ...first,
      session: {
        ...first.session,
        subtitles: {
          ...first.session.subtitles,
          source: { text: "draft two", isFinal: false },
          translation: { text: "preview", isFinal: false },
        },
      },
    };

    expect(
      Object.is(
        selectSessionStatusKind(first),
        selectSessionStatusKind(replacement),
      ),
    ).toBe(true);
    expect(
      Object.is(
        selectSessionErrorMessage(first),
        selectSessionErrorMessage(replacement),
      ),
    ).toBe(true);
    expect(
      Object.is(
        selectHasRecognizingSourceDraft(first),
        selectHasRecognizingSourceDraft(replacement),
      ),
    ).toBe(true);
  });

  it("still exposes a changed session error message to the tray", () => {
    const current = useStore.getState();
    const first = {
      ...current,
      session: {
        ...current.session,
        status: { kind: "error" as const, message: "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE" },
      },
    };
    const replacement = {
      ...first,
      session: {
        ...first.session,
        status: { kind: "error" as const, message: "audio3_error.setup.timeout.CLIENT_ERROR" },
      },
    };

    expect(selectSessionErrorMessage(first)).not.toBe(
      selectSessionErrorMessage(replacement),
    );
  });

  it("requires a new DeepL key in preview mode and never retains it in settings", async () => {
    const original = useStore.getState();
    const profile = { ...original.settings.profiles[0], provider: "alibabaCloud" as const, credentialState: "present" as const, textTranslation: "followService" as const };
    useStore.setState({
      settings: { ...original.settings, profiles: [profile], activeProfileId: profile.id },
      session: { ...original.session, isActive: false, status: { kind: "idle" } },
    });
    try {
      const credentials = { kind: "alibabaTranslation" as const, model: "", apiKey: "", textTranslation: "deepL" as const, endpoint: "", token: "" };
      await expect(useStore.getState().saveProfileCredentials(profile.id, credentials)).rejects.toThrow("credential-empty");
      const saved = await useStore.getState().saveProfileCredentials(profile.id, { ...credentials, token: "synthetic-deepl-key" });
      expect(saved.profiles[0].textTranslation).toBe("deepL");
      expect(JSON.stringify(saved)).not.toContain("synthetic-deepl-key");
      await expect(useStore.getState().saveProfileCredentials(profile.id, credentials)).resolves.toMatchObject({ profiles: [{ textTranslation: "deepL" }] });
    } finally {
      useStore.setState({ settings: original.settings, session: original.session });
    }
  });

  it("requires explicit third-party configuration and never includes it in preview snapshots", async () => {
    const original = useStore.getState();
    const profile = { ...original.settings.profiles[0], provider: "alibabaCloud" as const, credentialState: "present" as const, textTranslation: "followService" as const };
    useStore.setState({
      settings: { ...original.settings, profiles: [profile], activeProfileId: profile.id },
      session: { ...original.session, isActive: false, status: { kind: "idle" } },
    });
    try {
      const credentials = { kind: "alibabaTranslation" as const, apiKey: "", textTranslation: "openAICompatible" as const, endpoint: "https://synthetic.example/v1", token: "synthetic-translation-key", model: "synthetic-model" };
      for (const field of ["endpoint", "model"] as const) {
        await expect(useStore.getState().saveProfileCredentials(profile.id, { ...credentials, [field]: "" })).rejects.toThrow("credential-empty");
      }
      const saved = await useStore.getState().saveProfileCredentials(profile.id, credentials);
      expect(saved.profiles[0].textTranslation).toBe("openAICompatible");
      expect(JSON.stringify(saved)).not.toMatch(/synthetic-translation-key|synthetic-model|synthetic.example/);
      await expect(useStore.getState().saveProfileCredentials(profile.id, { ...credentials, endpoint: "", token: "", model: "new-model" })).resolves.toMatchObject({ profiles: [{ textTranslation: "openAICompatible" }] });
      await expect(useStore.getState().saveProfileCredentials(profile.id, { ...credentials, token: "" })).resolves.toMatchObject({ profiles: [{ textTranslation: "openAICompatible" }] });
    } finally {
      useStore.setState({ settings: original.settings, session: original.session });
    }
  });
});

it.each(["chatMock", "openAICompatible"] as const)("preview accepts keyless %s without changing speech credentials", async (textTranslation) => {
  const original = useStore.getState();
  const profile = { ...original.settings.profiles[0], provider: "alibabaCloud" as const, credentialState: "present" as const, textTranslation: "followService" as const };
  try {
    useStore.setState({ settings: { ...original.settings, profiles: [profile] }, session: { ...original.session, isActive: false, isPaused: false, status: { kind: "idle" } } });
    const credentials = { kind: "alibabaTranslation" as const, apiKey: "", textTranslation, endpoint: "http://127.0.0.1:8000/v1", model: "", token: "" };
    await expect(useStore.getState().saveProfileCredentials(profile.id, credentials)).rejects.toThrow("credential-empty");
    await useStore.getState().saveProfileCredentials(profile.id, { ...credentials, model: "synthetic-model" });
    expect(useStore.getState().settings.profiles[0]).toMatchObject({ textTranslation, credentialState: "present" });
  } finally { useStore.setState(original); }
});

it.each([false, true])("quick-switches Original and translation while preserving pause=%s and rejects transitions", async isPaused => {
  const original = useStore.getState();
  const session = { ...original.session, status: { kind: "listening" as const }, isActive: !isPaused, isPaused };
  try {
    useStore.setState({ session, settings: { ...original.settings, targetLanguage: "ja", sourceLanguage: "en", languageCapabilities: undefined,
      profiles: [{ id: "ali", name: "Alibaba", provider: "alibabaCloud", credentialState: "present" }], activeProfileId: "ali" } });
    await useStore.getState().switchTargetLanguage("original");
    expect(useStore.getState().settings.targetLanguage).toBe("original");
    expect(useStore.getState().session).toBe(session);
    useStore.setState({ settings: { ...useStore.getState().settings, sourceLanguage: "no" } });
    await useStore.getState().switchTargetLanguage("ja");
    expect(useStore.getState().settings.targetLanguage).toBe("ja");
    expect(useStore.getState().settings.sourceLanguage).toBe("auto");
    expect(useStore.getState().session).toBe(session);
    for (const kind of ["connecting", "stopping"] as const) {
      useStore.setState({ session: { ...session, status: { kind } } });
      await expect(useStore.getState().switchTargetLanguage("original")).rejects.toThrow("target_switch_busy");
      expect(useStore.getState().settings.targetLanguage).toBe("ja");
    }
    useStore.setState({ session, settings: { ...useStore.getState().settings,
      profiles: [{ id: "ali", name: "OpenAI", provider: "openAIRealtime", credentialState: "present" }] } });
    await expect(useStore.getState().switchTargetLanguage("original")).rejects.toThrow("target_switch_unsupported");
  } finally { useStore.setState(original, true); }
});

it("never renders an arbitrary provider error body as a session reason", () => {
  const state = useStore.getState();
  const message = selectSessionErrorMessage({ session: { ...state.session, status: { kind: "error", message: "private provider content sk-example" } } });
  expect(message).toBeTruthy();
  expect(message).not.toContain("private");
  expect(message).not.toContain("sk-example");
});

it("keeps arbitrary provider content out of the compact error cause", () => {
  const state = useStore.getState();
  const failed = { session: { ...state.session, status: { kind: "error" as const, message: "provider-private-content synthetic-key" } } };
  expect(selectSessionErrorSummary(failed)).toBe(selectSessionErrorMessage(failed));
  expect(selectSessionErrorSummary(failed)).not.toMatch(/provider-private-content|synthetic-key/);
  expect(selectSessionErrorSummary({ session: { ...state.session, status: { kind: "listening" } } })).toBeNull();
});
it("keeps custom declarations independent and normalizes only the active source in the browser preview", async () => {
  const original = useStore.getState();
  const profile = { id: "custom", name: "Custom", provider: "customDashScopeASR" as const, credentialState: "missing" as const, textTranslation: "openAICompatible" as const };
  try {
    useStore.setState({ settings: { ...original.settings, profiles: [profile], activeProfileId: profile.id, sourceLanguage: "fr", targetLanguage: "zh" } });
    const narrowed = await useStore.getState().updateProfile(profile.id, undefined, { customSpeechSourceLanguages: ["en", "en"] });
    expect(narrowed.sourceLanguage).toBe("auto");
    expect(narrowed.profiles[0].customSpeechSourceLanguages).toEqual(["en"]);
    await useStore.getState().updateProfile(profile.id, "Renamed");
    expect(useStore.getState().settings.profiles[0].customSpeechSourceLanguages).toEqual(["en"]);
    const cleared = await useStore.getState().updateProfile(profile.id, undefined, { customSpeechSourceLanguages: null });
    expect(cleared.profiles[0].customSpeechSourceLanguages).toBeNull();
    useStore.setState({ settings: original.settings });
    await expect(useStore.getState().updateProfile(original.settings.activeProfileId, undefined, { customSpeechSourceLanguages: [] })).rejects.toThrow("provider-mismatch");
  } finally { useStore.setState(original, true); }
});


it("rejects stale or busy recognition choices without changing preferences or subtitle state", async () => {
  const original = useStore.getState();
  try {
    const settings = { ...original.settings, sourceLanguage: "auto" as const, languageCapabilities: undefined,
      profiles: [{ id: "test", name: "Test", provider: "openAIRealtime" as const, credentialState: "present" as const }], activeProfileId: "test" };
    useStore.setState({ settings, session: original.session });
    await expect(useStore.getState().switchSourceLanguage("fr")).rejects.toThrow("source_switch_unsupported");
    expect(useStore.getState().settings).toBe(settings);
    expect(useStore.getState().session).toBe(original.session);
    for (const kind of ["connecting", "stopping"] as const) {
      const session = { ...original.session, status: { kind } };
      useStore.setState({ session });
      await expect(useStore.getState().switchSourceLanguage("auto")).rejects.toThrow("source_switch_busy");
      expect(useStore.getState().settings).toBe(settings);
      expect(useStore.getState().session).toBe(session);
    }
  } finally { useStore.setState(original, true); }
});


it.each([false, true])("selects a saved preview profile preserving subtitles, capture choice and pause=%s", async isPaused => {
  const original = useStore.getState();
  const other = { ...original.settings.profiles[0], id: "other" };
  const session = { ...original.session, isActive: !isPaused, isPaused, status: { kind: "listening" as const } };
  try {
    useStore.setState({ session, settings: { ...original.settings, profiles: [...original.settings.profiles, other], audioInput: "both", recordSessionAudio: true } });
    await useStore.getState().selectProfile(other.id);
    expect(useStore.getState().settings).toMatchObject({ activeProfileId: other.id, audioInput: "both", recordSessionAudio: true });
    expect(useStore.getState().session).toBe(session);
    await expect(useStore.getState().updateProfile(other.id, "Change")).rejects.toThrow("session-active");
  } finally { useStore.setState(original, true); }
});

it.each(["connecting", "stopping"] as const)("rejects preview profile selection while %s", async kind => {
  const original = useStore.getState();
  try {
    useStore.setState({ session: { ...original.session, status: { kind } } });
    await expect(useStore.getState().selectProfile("other")).rejects.toThrow("profile_switch_busy");
    expect(useStore.getState().settings).toBe(original.settings);
  } finally { useStore.setState(original, true); }
});


it("explicitly selects an Apple language even when its profile is already active", async () => {
  const original = useStore.getState();
  const apple = { ...original.settings.profiles[0], id: "apple", provider: "appleSpeech" as const };
  try {
    useStore.setState({ session: original.session, settings: { ...original.settings, profiles: [apple], activeProfileId: apple.id, sourceLanguage: "en", targetLanguage: "original" } });
    await useStore.getState().selectProfile(apple.id, "ja");
    expect(useStore.getState().settings).toMatchObject({ activeProfileId: apple.id, sourceLanguage: "ja" });
    const saved = useStore.getState().settings;
    await expect(useStore.getState().selectProfile(apple.id, "auto")).rejects.toThrow("apple_speech_language_unsupported");
    expect(useStore.getState().settings).toBe(saved);
    useStore.setState({ session: { ...original.session, isActive: true, status: { kind: "listening" } } });
    await expect(useStore.getState().selectProfile(apple.id, "en")).rejects.toThrow("session-active");
    expect(useStore.getState().settings).toBe(saved);
  } finally { useStore.setState(original, true); }
});
