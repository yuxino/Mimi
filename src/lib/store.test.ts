import { describe, expect, it } from "vitest";
import { isTauri } from "./ipc";
import {
  selectHasRecognizingSourceDraft,
  selectSessionErrorMessage,
  selectSessionStatusKind,
  useStore,
} from "./store";

describe("local preview store", () => {
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
        status: { kind: "error" as const, message: "first failure" },
      },
    };
    const replacement = {
      ...first,
      session: {
        ...first.session,
        status: { kind: "error" as const, message: "second failure" },
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
      for (const field of ["endpoint", "token", "model"] as const) {
        await expect(useStore.getState().saveProfileCredentials(profile.id, { ...credentials, [field]: "" })).rejects.toThrow("credential-empty");
      }
      const saved = await useStore.getState().saveProfileCredentials(profile.id, credentials);
      expect(saved.profiles[0].textTranslation).toBe("openAICompatible");
      expect(JSON.stringify(saved)).not.toMatch(/synthetic-translation-key|synthetic-model|synthetic.example/);
      await expect(useStore.getState().saveProfileCredentials(profile.id, { ...credentials, endpoint: "", token: "", model: "new-model" })).resolves.toMatchObject({ profiles: [{ textTranslation: "openAICompatible" }] });
      await expect(useStore.getState().saveProfileCredentials(profile.id, { ...credentials, token: "" })).rejects.toThrow("credential-empty");
    } finally {
      useStore.setState({ settings: original.settings, session: original.session });
    }
  });
});
