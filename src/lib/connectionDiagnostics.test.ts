import { audio3ErrorMessage } from "./audio3Errors";
import { afterEach, expect, it } from "vitest";
import { I18N, setStoredUiLanguage } from "./i18n";
import { sessionErrorSettingsTarget, sessionActionErrorMessage, languageActionErrorMessage, connectionDiagnosticMessage, credentialErrorMessage, credentialUnavailableHelp, profileErrorMessage, diagnosticCopy, diagnosticPlatform } from "./connectionDiagnostics";

it.each(["zh", "en", "ja"] as const)("gives Tencent activation, quota and concurrency recovery without provider text in %s", language => {
  setStoredUiLanguage(language);
  const reasons = {
    tencent_configuration_rejected: "invalidConfiguration",
    tencent_service_activation_required: "serviceNotActivated",
    tencent_quota_exhausted: "quotaExhausted",
    tencent_capacity_exceeded: "concurrencyLimited",
    tencent_provider_rejected: "serviceRejected",
  } as const;
  for (const [label, reason] of Object.entries(reasons)) {
    expect(credentialErrorMessage(label)).toBe(diagnosticCopy().reasons[reason]);
    expect(connectionDiagnosticMessage({ credential: "present", service: "unavailable", reason }))
      .toContain(diagnosticCopy().reasons[reason]);
    expect(credentialErrorMessage(`${label}: private-provider-body`)).toBeNull();
  }
  for (const label of ["tencent_configuration_rejected", "tencent_service_activation_required", "tencent_quota_exhausted"])
    expect(sessionErrorSettingsTarget(label)).toBe("service");
  expect(sessionErrorSettingsTarget("tencent_capacity_exceeded")).toBeNull();
});

it.each(["zh", "en", "ja"] as const)("gives Apple resource and language recovery in %s without exposing runtime labels", language => {
  setStoredUiLanguage(language);
  expect(credentialErrorMessage("apple_speech_assets_missing")).toBe(I18N.settings.appleSpeechAssetsMissing);
  expect(profileErrorMessage("apple_speech_language_unsupported")).toBe(I18N.settings.appleSpeechLanguageUnsupported);
  expect(profileErrorMessage("apple_speech_translation_language_unsupported")).toBe(I18N.settings.appleSpeechTranslationLanguageUnsupported);
  expect(I18N.settings.appleSpeechAssetsMissing).toContain(I18N.settings.appleSpeechDownloadAndUse);
  expect(credentialErrorMessage("apple_speech_unavailable")).toBe(I18N.settings.appleSpeechUnavailable);
  for (const suffix of ["setup_timeout", "start_failed", "recognition_failed", "audio_failed", "not_connected", "result_backlog", "invalid_result", "finalize_timeout"]) {
    expect(credentialErrorMessage(`apple_speech_${suffix}`)).toBe(I18N.settings.appleSpeechRecognitionFailed);
  }
  expect(profileErrorMessage("apple_speech_prepare_failed: private-native-path")).not.toContain("private-native-path");
});

it.each(["zh", "en", "ja"] as const)("keeps Apple translation recovery separate from speech packs and private in %s", language => {
  setStoredUiLanguage(language);
  const errors = {
    apple_translation_assets_missing: I18N.settings.appleTranslationAssetsMissing,
    apple_translation_language_unsupported: I18N.settings.appleTranslationUnsupported,
    apple_translation_unavailable: I18N.settings.appleTranslationUnavailable,
    apple_translation_cancelled: I18N.settings.appleTranslationCancelled,
    apple_translation_busy: I18N.settings.appleTranslationBusy,
    apple_translation_timeout: I18N.settings.appleTranslationFailed,
    apple_translation_invalid_input: I18N.settings.appleTranslationFailed,
    apple_translation_invalid_result: I18N.settings.appleTranslationFailed,
    apple_translation_failed: I18N.settings.appleTranslationFailed,
  };
  for (const [label, message] of Object.entries(errors)) {
    expect(credentialErrorMessage(label)).toBe(message);
    expect(sessionActionErrorMessage(new Error(label), "fallback")).toBe(message);
    expect(credentialErrorMessage(`${label}: private-native-content`)).toBeNull();
    expect(sessionErrorSettingsTarget(`${label}: private-native-content`)).toBeNull();
  }
  for (const label of ["apple_translation_assets_missing", "apple_translation_language_unsupported", "apple_translation_unavailable"]) {
    expect(sessionErrorSettingsTarget(label)).toBe("service");
  }
  expect(sessionErrorSettingsTarget("apple_speech_assets_missing")).toBe("appleSpeechResources");
  for (const reason of ["appleTranslationAssetsMissing", "appleTranslationLanguageUnsupported", "appleTranslationUnavailable", "appleTranslationFailed"] as const) {
    expect(diagnosticCopy().reasons[reason]).toBeTruthy();
  }
});


it.each(["zh", "en", "ja"] as const)("keeps every Apple readiness failure specific across language, profile and session actions in %s", language => {
  setStoredUiLanguage(language);
  const messages = {
    apple_speech_assets_missing: I18N.settings.appleSpeechAssetsMissing,
    apple_speech_language_unsupported: I18N.settings.appleSpeechLanguageUnsupported,
    apple_speech_translation_language_unsupported: I18N.settings.appleSpeechTranslationLanguageUnsupported,
    apple_speech_unavailable: I18N.settings.appleSpeechUnavailable,
    apple_speech_ui_test_unavailable: I18N.settings.appleSpeechUnavailable,
    apple_speech_status_failed: I18N.settings.appleSpeechLoadFailed,
    apple_speech_prepare_failed: I18N.settings.appleSpeechPrepareFailed,
    apple_speech_service_unavailable: I18N.settings.appleSpeechServiceUnavailable,
    apple_speech_reservation_limit: I18N.settings.appleSpeechReservationLimit,
    apple_speech_download_cancelled: I18N.settings.appleSpeechDownloadCancelled,
    apple_speech_download_timeout: I18N.settings.appleSpeechDownloadTimeout,
    apple_speech_download_network: I18N.settings.appleSpeechDownloadNetwork,
    apple_speech_download_storage: I18N.settings.appleSpeechDownloadStorage,
    apple_speech_resources_unavailable: I18N.settings.appleSpeechResourcesUnavailable,
    apple_speech_preparing: I18N.settings.appleSpeechPreparationInProgress,
  };
  for (const [label, expected] of Object.entries(messages)) {
    expect(credentialErrorMessage(label)).toBe(expected);
    for (const error of [label, new Error(label)]) {
      expect(languageActionErrorMessage(error, "fallback")).toBe(expected);
      expect(profileErrorMessage(error)).toBe(expected);
      expect(sessionActionErrorMessage(error, "fallback")).toBe(expected);
    }
    for (const privateError of [`${label}: private-native-path`, `private-provider-body ${label}`, `${label}_unknown`]) {
      expect(credentialErrorMessage(privateError)).toBeNull();
      expect(languageActionErrorMessage(new Error(privateError), "fallback")).toBe("fallback");
      expect(profileErrorMessage(privateError)).toBe(I18N.settings.profileActionFailed);
      expect(sessionActionErrorMessage(privateError, "fallback")).toBe("fallback");
    }
  }
  for (const message of [I18N.settings.appleSpeechLanguageUnsupported,
    I18N.settings.appleSpeechTranslationLanguageUnsupported, I18N.settings.appleSpeechUnavailable]) {
    expect(message).toContain(I18N.settings.serviceProfilesTitle);
    expect(message).not.toBe(I18N.overlay.controlActionFailed);
  }
  // Download failures keep their retry/wait instruction beside the action;
  // they must not tell a user already in this editor to find it again.
  expect(I18N.settings.appleSpeechPrepareFailed).not.toBe(I18N.overlay.controlActionFailed);
  expect(I18N.settings.appleSpeechPreparationInProgress).not.toBe(I18N.overlay.controlActionFailed);
  expect(sessionActionErrorMessage({ message: "apple_speech_assets_missing" }, "fallback")).toBe("fallback");
});

it("localizes exhausted translation recovery without showing internal labels", () => {
  for (const language of ["zh", "en", "ja"] as const) {
    setStoredUiLanguage(language);
    expect(credentialErrorMessage("translation_rate_limited")).toBe(diagnosticCopy().translationLimited);
    expect(credentialErrorMessage("translation_temporarily_unavailable")).toBe(diagnosticCopy().translationTemporary);
    expect(credentialErrorMessage("translation_source_unsupported")).toBe(diagnosticCopy().translationSourceUnsupported);
    expect(credentialErrorMessage("subtitle_text_too_large")).toBe(diagnosticCopy().subtitleTooLarge);
    expect(credentialErrorMessage("The subtitle service returned too much text.")).toBe(diagnosticCopy().subtitleTooLarge);
    expect(credentialErrorMessage("Translation fell behind live audio. mimi is reconnecting.")).toBe(diagnosticCopy().translationBacklog);
  }
});
import type { ConnectionDiagnostic } from "./ipc";
afterEach(() => setStoredUiLanguage("en"));
it.each(["en", "zh", "ja"] as const)("explains dev file credentials without suggesting Keychain authorization in %s", (language) => {
  setStoredUiLanguage(language);
  expect(profileErrorMessage("local_dev_credentials_read_only")).toBe(diagnosticCopy().localDevReadOnly);
  expect(profileErrorMessage("local_dev_credentials_unavailable")).toBe(diagnosticCopy().localDevUnavailable);
  expect(profileErrorMessage("local_dev_credentials_read_only")).toContain(".env");
  expect(profileErrorMessage("local_dev_credentials_unavailable")).toContain("0600");
  expect(connectionDiagnosticMessage({ credential: "localDevUnavailable", service: "unavailable", reason: "localDevCredentialsUnavailable" })).toContain(diagnosticCopy().localDevUnavailable);
  expect(profileErrorMessage("local_dev_credentials_unavailable: synthetic-private-value")).not.toContain("synthetic-private-value");
});
it("localizes shortcut and storage errors without losing recovery guidance", () => {
  setStoredUiLanguage("zh");
  expect(credentialErrorMessage("credential_store_unavailable", "macos")).toContain("无法读取服务凭据");
  expect(credentialErrorMessage("The system credential store is unavailable.")).not.toContain("The system");
  expect(credentialErrorMessage("credential_authentication_failed")).toContain("服务拒绝了认证");
});
it("never interpolates arbitrary native errors or synthetic secrets", () => {
  expect(profileErrorMessage("synthetic-secret-private-value")).not.toContain("synthetic-secret");
});

it("localizes sanitized third-party translation errors without provider response content", () => {
  for (const language of ["zh", "en", "ja"] as const) {
    setStoredUiLanguage(language);
    expect(profileErrorMessage("The OpenAI-compatible endpoint is invalid.")).toBe(I18N.settings.deepLXEndpointInvalid);
    expect(profileErrorMessage("Use an HTTPS OpenAI-compatible endpoint (or HTTP on localhost), without URL credentials, query or fragment.")).toBe(I18N.settings.deepLXEndpointInvalid);
    expect(profileErrorMessage("The OpenAI-compatible model name is invalid.")).toBe(I18N.settings.openAICompatibleModelInvalid);
    expect(credentialErrorMessage("Enter a valid API key for the custom translation service.")).toBe(I18N.settings.openAICompatibleApiKeyInvalid);
    expect(credentialErrorMessage("The custom translation service rejected the request (HTTP 401). Check its API key, model and service address. synthetic-private-value")).toContain("401");
    expect(credentialErrorMessage("The custom translation service rejected the request (HTTP 401). Check its API key, model and service address. synthetic-private-value")).not.toContain("synthetic-private-value");
    for (const error of ["The custom translation service timed out.", "Could not connect to the custom translation service.", "The custom translation service returned invalid or empty text.", "The custom translation service returned too much data."]) {
      expect(credentialErrorMessage(error)).not.toBeNull();
      expect(credentialErrorMessage(error + " synthetic-private-value")).not.toContain("synthetic-private-value");
    }
    expect(credentialErrorMessage("The selected language is not supported by this custom translation route.")).toBe(diagnosticCopy().translationSourceUnsupported);
  }
});
it("shows a short unavailable reason instead of a reachability disclaimer", () => {
  setStoredUiLanguage("zh");
  const message = connectionDiagnosticMessage({ credential: "missing", service: "unavailable", reason: "credentialsMissing" });
  expect(message).toBe("不可用: 请先保存凭据。");
  expect(message).not.toContain("HTTP");
  expect(message).not.toContain("认证成功");
});
it("localizes every service failure reason and explains skipped preview-mode checks", () => {
  const reasons = ["credentialsMissing", "credentialsUnavailable", "credentialsServiceUnavailable", "credentialsAccessDenied", "textTranslationNotConfigured", "invalidConfiguration", "unsupportedLanguage", "localRecognitionOverloaded", "localRecognitionTimeout", "authenticationRejected", "serviceRejected", "timeout", "unreachable", "appleSpeechAssetsMissing", "appleSpeechPreparing", "appleSpeechStatusFailed", "appleSpeechLanguageUnsupported", "appleSpeechUnavailable", "appleSpeechRecognitionFailed"] as const;
  for (const language of ["zh", "en", "ja"] as const) {
    setStoredUiLanguage(language);
    for (const reason of reasons) {
      expect(connectionDiagnosticMessage({ credential: "present", service: "unavailable", reason })).toBe(`${diagnosticCopy().unavailable}: ${diagnosticCopy().reasons[reason]}`);
    }
    expect(connectionDiagnosticMessage({ credential: "present", service: "notTested", reason: null })).toBe(diagnosticCopy().checkSkipped);
    expect(connectionDiagnosticMessage({ credential: "present", service: "available", reason: null })).toBe(diagnosticCopy().available);
  }
});

it("shows a short endpoint correction instead of the whole provider description", () => {
  for (const language of ["en", "zh", "ja"] as const) {
    setStoredUiLanguage(language);
    const message = profileErrorMessage("Use an HTTPS DeepLX endpoint (or HTTP on localhost), without URL credentials, query or fragment.");
    expect(message).toContain("HTTPS");
    expect(message).toContain("localhost");
    expect(message).toContain("?");
    expect(message).toContain("#");
    expect(message).not.toContain("Audio 3.0");
  }
});

it("never upgrades the former network-only contract to available", () => {
  const legacy = { credential: "present", network: "reachable" } as unknown as ConnectionDiagnostic;
  expect(connectionDiagnosticMessage(legacy)).toBe(diagnosticCopy().checkFailed);
});

it("limits short storage recovery to the detected platform", () => {
  setStoredUiLanguage("zh");
  expect(diagnosticPlatform("Mozilla Mac OS X")).toBe("macos");
  expect(diagnosticPlatform("Mozilla Windows NT")).toBe("windows");
  expect(diagnosticPlatform("Mozilla Linux")).toBe("linux");
  expect(diagnosticCopy("macos").reasons.credentialsAccessDenied).toContain("钥匙串");
  expect(diagnosticCopy("windows").reasons.credentialsAccessDenied).toContain("凭据管理器");
  expect(diagnosticCopy("linux").reasons.credentialsAccessDenied).toContain("密码存储");
  for (const platform of ["macos", "windows", "linux"] as const) {
    expect(diagnosticCopy(platform).reasons.credentialsAccessDenied).not.toMatch(/HTTP|Debian|测试模式/);
  }
});

it.each([
  ["en", "Install or enable", "Sign out and back in", "locked or access was denied", "Unlock it or allow", "No credentials configured"],
  ["zh", "安装或启用", "注销并重新登录", "已锁定或访问被拒绝", "解锁密码存储或允许", "尚未配置凭据"],
  ["ja", "インストールするか有効", "ログアウトして再ログイン", "ロックされているか、アクセスが拒否", "解除するかシステムのアクセス許可を承認", "認証情報が未設定"],
] as const)("distinguishes missing Linux services, denied access and missing credentials in %s", (language, setup, login, denied, unlock, missing) => {
  setStoredUiLanguage(language);
  const service = credentialErrorMessage("credential_service_unavailable", "linux")!;
  expect(service).toContain("Secret Service");
  expect(service).toContain("GNOME Keyring");
  expect(service).toContain(setup);
  expect(service).toContain(login);
  expect(service).not.toContain(unlock);
  const labels = diagnosticCopy("linux");
  expect(connectionDiagnosticMessage({ credential: "serviceUnavailable", service: "unavailable", reason: "credentialsServiceUnavailable" }, "linux")).toBe(`${labels.unavailable}: ${labels.reasons.credentialsServiceUnavailable}`);

  const access = credentialErrorMessage("credential_store_access_denied", "linux")!;
  expect(access).toContain(denied);
  expect(access).toContain(unlock);
  expect(access).not.toContain(setup);
  expect(connectionDiagnosticMessage({ credential: "accessDenied", service: "unavailable", reason: "credentialsAccessDenied" }, "linux")).toBe(`${labels.unavailable}: ${labels.reasons.credentialsAccessDenied}`);
  expect(labels.missing).toContain(missing);
  expect(connectionDiagnosticMessage({ credential: "missing", service: "unavailable", reason: "credentialsMissing" }, "linux")).toBe(`${labels.unavailable}: ${labels.reasons.credentialsMissing}`);
  expect(labels.reasons.credentialsServiceUnavailable).not.toBe(labels.reasons.credentialsAccessDenied);
});

it("uses local file recovery for ordinary storage errors on every platform", () => {
  setStoredUiLanguage("en");
  expect(diagnosticCopy("linux").storage).toContain("local file access");
  expect(credentialErrorMessage("credential_store_unavailable", "linux")).toBe(diagnosticCopy("linux").storage);
  expect(connectionDiagnosticMessage({ credential: "unavailable", service: "unavailable", reason: "credentialsUnavailable" }, "linux")).toContain(diagnosticCopy("linux").reasons.credentialsUnavailable);
  expect(diagnosticCopy("macos").storage).toBe("Cannot read service credentials. Check local file access or save them again.");
  expect(diagnosticCopy("macos").storage).not.toContain("Secret Service");
  expect(credentialUnavailableHelp("linux")).toBe(diagnosticCopy("linux").storage);
  expect(credentialUnavailableHelp("macos")).toBe(diagnosticCopy("macos").storage);
  expect(credentialUnavailableHelp("windows")).toBe(diagnosticCopy("windows").storage);
});

it("shortens the known quota prefix without exposing trailing provider text or URLs", () => {
  const error = "You exceeded your current quota, please check your plan and billing details. https://help.aliyun.com/synthetic-quota-help";
  for (const language of ["en", "zh", "ja"] as const) {
    setStoredUiLanguage(language);
    expect(credentialErrorMessage(error)).toBe(diagnosticCopy().quota);
    expect(credentialErrorMessage(error)).not.toMatch(/https|billing|balance|余额/);
  }
  expect(credentialErrorMessage("Unknown quota response with synthetic-private-value")).toBeNull();
  expect(credentialErrorMessage("You exceeded your current quota; synthetic different response")).toBeNull();
});

it.each(["en", "zh", "ja"] as const)("keeps credential recovery actionable even when the provider check was skipped in %s", language => {
  setStoredUiLanguage(language);
  for (const reason of ["credentialsMissing", null] as const) {
    const message = connectionDiagnosticMessage({ credential: "missing", service: "notTested", reason });
    expect(message).toContain(diagnosticCopy().reasons.credentialsMissing);
    expect(message).not.toBe(diagnosticCopy().checkSkipped);
    expect(message).not.toContain(diagnosticCopy().notTested);
  }
});

it.each(["en", "zh", "ja"] as const)("ordinary file errors do not recommend native authorization in %s", (language) => {
  setStoredUiLanguage(language);
  for (const platform of ["macos", "windows", "linux"] as const) {
    expect(credentialErrorMessage("credential_store_unavailable", platform)).not.toMatch(/Keychain|Keyring|Secret Service|Credential Manager|钥匙串|凭据管理器|キーチェーン/);
    expect(connectionDiagnosticMessage({ credential: "unavailable", service: "unavailable", reason: "credentialsUnavailable" }, platform)).not.toMatch(/Keychain|Keyring|Secret Service|Credential Manager|钥匙串|凭据管理器|キーチェーン/);
  }
});

it.each(["en", "zh", "ja"] as const)("explains fixed Audio3 connection failures without exposing transport details in %s", language => {
  setStoredUiLanguage(language);
  const unreachable = [
    "The speech recognition transport failed.",
    "The speech recognition session is not connected.",
    "The speech recognition connection closed.",
  ];
  const timeouts = [
    "The speech recognition connection could not be established in time.",
    "The speech recognition connection stopped responding.",
  ];
  for (const error of unreachable) expect(credentialErrorMessage(error)).toBe(diagnosticCopy().speechUnreachable);
  for (const error of timeouts) expect(credentialErrorMessage(error)).toBe(diagnosticCopy().speechTimeout);
  expect(diagnosticCopy().speechUnreachable).not.toBe(diagnosticCopy().unreachable);
  for (const error of [...unreachable, ...timeouts]) {
    expect(credentialErrorMessage(`${error} ws://synthetic-private-endpoint`)).toBeNull();
  }
  expect(credentialErrorMessage("IO error: connection refused at synthetic-private-endpoint")).toBeNull();
});

it.each(["zh", "en", "ja"] as const)("explains known language failures without exposing arbitrary IPC text in %s", locale => {
  setStoredUiLanguage(locale);
  const messages = {
    source_switch_busy: I18N.settings.languageSwitchBusy,
    target_switch_busy: I18N.settings.languageSwitchBusy,
    source_switch_superseded: I18N.settings.languageSwitchSuperseded,
    language_switch_superseded: I18N.settings.languageSwitchSuperseded,
    source_switch_unsupported: I18N.settings.languageSwitchUnsupported,
    target_switch_unsupported: I18N.settings.languageSwitchUnsupported,
    source_switch_save_failed: I18N.settings.languageSaveFailed,
    source_switch_profile: I18N.settings.languageSwitchProfileUnavailable,
  };
  for (const [label, expected] of Object.entries(messages)) {
    expect(languageActionErrorMessage(label, "fallback")).toBe(expected);
    expect(languageActionErrorMessage(new Error(label), "fallback")).toBe(expected);
  }
  expect(languageActionErrorMessage("Listening settings cannot be changed while a session is active.", "fallback")).toBe(I18N.settings.languageChangeRequiresStop);
  expect(languageActionErrorMessage(new Error("custom_speech_unreachable"), "fallback")).toBe(I18N.settings.customSpeechUnreachable);
  expect(languageActionErrorMessage("synthetic-private-provider-body", "fallback")).toBe("fallback");
});

it.each(["en", "zh", "ja"] as const)("localizes profile switch restrictions without raw payloads in %s", language => {
  setStoredUiLanguage(language);
  expect(profileErrorMessage("profile_switch_recording_requires_stop")).toBe(I18N.settings.profileSwitchRecordingRequiresStop);
  expect(profileErrorMessage(new Error("profile_switch_busy"))).toBe(I18N.settings.profileSwitchBusy);
  expect(profileErrorMessage("profile_switch_superseded")).toBe(I18N.settings.profileSwitchBusy);
  expect(profileErrorMessage("profile_switch_recording_requires_stop: private-value")).toBe(I18N.settings.profileActionFailed);
});


it.each(["en", "zh", "ja"] as const)("keeps post-save reconnect failures actionable and private in %s", language => {
  setStoredUiLanguage(language);
  const labels = [
    "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE",
    "audio3_error.setup.authentication.INVALID_API_KEY",
  ];
  for (const label of labels) {
    for (const error of [label, new Error(label)]) {
      expect(profileErrorMessage(error)).toBe(audio3ErrorMessage(label));
      expect(languageActionErrorMessage(error, "fallback")).toBe(audio3ErrorMessage(label));
    }
    expect(profileErrorMessage(`${label}: private-provider-body`)).toBe(I18N.settings.profileActionFailed);
    expect(languageActionErrorMessage(`${label}: private-provider-body`, "fallback")).toBe("fallback");
  }
  expect(profileErrorMessage(new Error("The speech recognition transport failed."))).toBe(diagnosticCopy().speechUnreachable);
});


it.each(["en", "zh", "ja"] as const)("sanitizes paused-session recovery errors in %s", language => {
  setStoredUiLanguage(language);
  const error = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
  expect(sessionActionErrorMessage(new Error(error), "fallback")).toBe(audio3ErrorMessage(error));
  expect(sessionActionErrorMessage("apple_speech_assets_missing", "fallback")).toBe(I18N.settings.appleSpeechAssetsMissing);
  expect(sessionActionErrorMessage(`${error}: private-provider-body`, "fallback")).toBe("fallback");
  expect(sessionActionErrorMessage(new Error("private-provider-body"), "fallback")).toBe("fallback");
});


it("routes only exact resource errors to Apple preparation and preserves other recovery behavior", () => {
  for (const label of [
    "apple_speech_assets_missing", "apple_speech_preparing", "apple_speech_prepare_failed",
    "apple_speech_service_unavailable", "apple_speech_reservation_limit", "apple_speech_download_cancelled",
    "apple_speech_download_timeout", "apple_speech_download_network", "apple_speech_download_storage",
    "apple_speech_resources_unavailable",
  ]) {
    expect(sessionErrorSettingsTarget(label)).toBe("appleSpeechResources");
    expect(sessionErrorSettingsTarget(new Error(label))).toBe("appleSpeechResources");
    expect(sessionErrorSettingsTarget(`${label}: private-runtime-detail`)).toBeNull();
    expect(sessionErrorSettingsTarget(`prefix ${label}`)).toBeNull();
    expect(sessionErrorSettingsTarget({ message: label })).toBeNull();
  }
  expect(sessionErrorSettingsTarget("audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE")).toBe("service");
  for (const label of ["apple_speech_recognition_failed", "apple_speech_unavailable", "apple_speech_language_unsupported", "audio3_error.recognition.timeout.LOCAL_TIMEOUT", "unknown"]) {
    expect(sessionErrorSettingsTarget(label)).toBeNull();
  }
});


it.each(["en", "zh", "ja"] as const)("keeps safe provider failures actionable across session, language and profile actions in %s", language => {
  setStoredUiLanguage(language);
  const labels = diagnosticCopy();
  const rejected = `${labels.reasons.serviceRejected} ${labels.reasons.invalidConfiguration}`;
  const cases: [string, string][] = [];
  for (const provider of ["OpenAI Realtime Translation", "Gemini Live Translation", "Azure OpenAI Realtime Translation", "xAI Grok Voice", "Tencent Cloud realtime translation", "Baidu realtime translation"]) {
    cases.push(
      [`The ${provider} connection failed.`, labels.unreachable],
      [`The ${provider} session is not connected.`, labels.unreachable],
      [`The ${provider} connection stopped responding.`, labels.timeout],
      [`${provider} rejected the session configuration.`, rejected],
      [`${provider} rejected the session.`, rejected],
      [`${provider} returned an invalid response.`, labels.translationTemporary],
      [`${provider} requires a translated output language.`, I18N.settings.languageSwitchUnsupported],
    );
    const setup = provider === "Tencent Cloud realtime translation" || provider === "Baidu realtime translation"
      ? "session" : "session configuration";
    cases.push([`${provider} did not confirm the ${setup} in time.`, labels.timeout]);
  }
  cases.push(
    ["The live translation transport failed.", labels.unreachable],
    ["The live translation session is not connected.", labels.unreachable],
    ["The live translation connection closed.", labels.unreachable],
    ["The live translation connection could not be established in time.", labels.timeout],
    ["The live translation connection stopped responding.", labels.timeout],
    ["The live translation session setup timed out.", labels.timeout],
    ["The live translation session setup was rejected.", rejected],
    ["The live translation service returned invalid data.", labels.translationTemporary],
    ["The Volcano Engine connection failed.", labels.unreachable],
    ["The Volcano Engine transport failed.", labels.unreachable],
    ["The Volcano Engine translation session is not connected.", labels.unreachable],
    ["The Volcano Engine connection stopped responding.", labels.timeout],
    ["The Volcano Engine connection could not be established in time.", labels.timeout],
    ["Volcano Engine did not confirm the session configuration in time.", labels.timeout],
    ["Volcano Engine rejected the session configuration.", rejected],
    ["Volcano Engine rejected the translation session.", rejected],
    ["Volcano Engine returned an invalid response.", labels.translationTemporary],
    ["Volcano Engine ended the translation session unexpectedly.", labels.translationTemporary],
    ["Volcano Engine requires an explicit Chinese, English, or Japanese source language.", I18N.settings.languageSwitchUnsupported],
    ["Volcano Engine requires a Chinese, English, or Japanese translation language.", I18N.settings.languageSwitchUnsupported],
    ["Baidu realtime translation requires an explicit supported source language.", I18N.settings.languageSwitchUnsupported],
    ["Baidu realtime translation ended the session unexpectedly.", labels.translationTemporary],
    ["Tencent Cloud realtime translation ended the session unexpectedly.", labels.translationTemporary],
    ["xAI Grok Voice did not finish the final turn in time.", labels.timeout],
    ["xAI Grok Voice did not complete the current turn.", labels.translationTemporary],
    ["credential_authentication_failed", labels.auth],
    ["custom_speech_authentication_failed", labels.auth],
    ["custom_speech_credentials_missing", labels.missing],
    ["custom_speech_endpoint_invalid", I18N.settings.customSpeechEndpointInvalid],
    ["custom_speech_model_invalid", I18N.settings.customSpeechModelInvalid],
    ["text_translation_credentials_missing", labels.missing],
  );
  for (const error of [
    "Add an Alibaba Cloud Model Studio API key in Settings.",
    "Add an OpenAI API key in Settings.",
    "Add a Google Gemini API key in Settings.",
    "Add an Azure OpenAI API key in Settings.",
    "Add an xAI API key in Settings.",
    "Add a Tencent Cloud AppID, SecretID, and SecretKey in Settings.",
    "Add a Baidu Cloud AppID and AppKey in Settings.",
    "Add a Volcano Engine API key in Settings.",
  ]) cases.push([error, labels.missing]);
  for (const error of [
    "Enter a valid Azure OpenAI resource endpoint in Settings.",
    "Enter Azure OpenAI translation and transcription deployment names in Settings.",
    "The Azure OpenAI endpoint must be an official HTTPS resource endpoint.",
    "The Azure OpenAI deployment name is invalid.",
  ]) cases.push([error, labels.reasons.invalidConfiguration]);
  for (const [label, expected] of cases) {
    expect(credentialErrorMessage(label), label).toBe(expected);
    const target = [labels.auth, labels.missing, labels.reasons.invalidConfiguration, I18N.settings.languageSwitchUnsupported, I18N.settings.customSpeechEndpointInvalid, I18N.settings.customSpeechModelInvalid].includes(expected) ? "service" : null;
    for (const error of [label, new Error(label)]) {
      expect(sessionErrorSettingsTarget(error), label).toBe(target);
      expect(languageActionErrorMessage(error, "fallback"), label).toBe(expected);
      expect(profileErrorMessage(error), label).toBe(expected);
      expect(sessionActionErrorMessage(error, "fallback"), label).toBe(expected);
    }
    for (const unsafe of [`${label} https://synthetic-private-endpoint?token=private`, `private-provider-body ${label}`, `${label}\n`]) {
      expect(sessionErrorSettingsTarget(unsafe)).toBeNull();
      expect(credentialErrorMessage(unsafe)).toBeNull();
      expect(languageActionErrorMessage(new Error(unsafe), "fallback")).toBe("fallback");
      expect(profileErrorMessage(unsafe)).toBe(I18N.settings.profileActionFailed);
      expect(sessionActionErrorMessage(unsafe, "fallback")).toBe("fallback");
    }
  }
  expect(credentialErrorMessage("The unknown provider connection failed.")).toBeNull();
  expect(credentialErrorMessage("Baidu realtime translation rejected the session: private-provider-body")).toBeNull();
  expect(sessionActionErrorMessage({ message: "The Baidu realtime translation connection failed." }, "fallback")).toBe("fallback");
});

it.each(["en", "zh", "ja"] as const)("retains only a canonical bounded Baidu setup code in %s", language => {
  setStoredUiLanguage(language);
  const labels = diagnosticCopy();
  const rejected = `${labels.reasons.serviceRejected} ${labels.reasons.invalidConfiguration}`;
  for (const code of ["0", "31003", "-1", "9223372036854775807", "-9223372036854775808"]) {
    const error = `Baidu realtime translation rejected the session configuration (code ${code}).`;
    const expected = `${rejected} (${code})`;
    expect(sessionErrorSettingsTarget(error)).toBeNull();
    expect(credentialErrorMessage(error)).toBe(expected);
    expect(languageActionErrorMessage(new Error(error), "fallback")).toBe(expected);
    expect(profileErrorMessage(error)).toBe(expected);
    expect(sessionActionErrorMessage(error, "fallback")).toBe(expected);
  }
  for (const code of ["9223372036854775808", "-9223372036854775809", "99999999999999999999", "-0", "01", "+1", "1.5", "1e3", "31003 private-token", "https://private-endpoint"]) {
    expect(credentialErrorMessage(`Baidu realtime translation rejected the session configuration (code ${code}).`)).toBeNull();
  }
  const exact = "Baidu realtime translation rejected the session configuration (code 31003).";
  for (const unsafe of [`${exact}\n`, `${exact}\r\n`, `${exact} private-token`, `private-token ${exact}`, exact.replace("31003", "31003) private-token ("), "Other service rejected the session configuration (code 31003). "]) {
    expect(credentialErrorMessage(unsafe)).toBeNull();
    expect(languageActionErrorMessage(unsafe, "fallback")).toBe("fallback");
    expect(sessionActionErrorMessage(unsafe, "fallback")).toBe("fallback");
    expect(profileErrorMessage(unsafe)).toBe(I18N.settings.profileActionFailed);
  }
});
