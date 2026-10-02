import { afterEach, expect, it } from "vitest";
import { I18N, setStoredUiLanguage } from "./i18n";
import { connectionDiagnosticMessage, credentialErrorMessage, credentialUnavailableHelp, profileErrorMessage, diagnosticCopy, diagnosticPlatform } from "./connectionDiagnostics";

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
it("localizes every service failure reason and leaves untested availability neutral", () => {
  const reasons = ["credentialsMissing", "credentialsUnavailable", "credentialsServiceUnavailable", "credentialsAccessDenied", "invalidConfiguration", "authenticationRejected", "serviceRejected", "timeout", "unreachable"] as const;
  for (const language of ["zh", "en", "ja"] as const) {
    setStoredUiLanguage(language);
    for (const reason of reasons) {
      expect(connectionDiagnosticMessage({ credential: "present", service: "unavailable", reason })).toBe(`${diagnosticCopy().unavailable}: ${diagnosticCopy().reasons[reason]}`);
    }
    expect(connectionDiagnosticMessage({ credential: "present", service: "notTested", reason: null })).toBe(diagnosticCopy().notTested);
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
  expect(connectionDiagnosticMessage(legacy)).toBe(diagnosticCopy().notTested);
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

it("gives general Linux storage errors provider setup guidance while preserving Mac unlock copy", () => {
  setStoredUiLanguage("en");
  expect(diagnosticCopy("linux").storage).toContain("installed and enabled in this desktop session");
  expect(credentialErrorMessage("credential_store_unavailable", "linux")).toBe(diagnosticCopy("linux").storage);
  expect(connectionDiagnosticMessage({ credential: "unavailable", service: "unavailable", reason: "credentialsUnavailable" }, "linux")).toContain(diagnosticCopy("linux").reasons.credentialsUnavailable);
  expect(diagnosticCopy("macos").storage).toBe("Cannot read service credentials. Handle the system unlock prompt, then check again.");
  expect(diagnosticCopy("macos").storage).not.toContain("Secret Service");
  expect(credentialUnavailableHelp("linux")).toBe(diagnosticCopy("linux").storage);
  expect(credentialUnavailableHelp("macos")).toBe(I18N.settings.credentialUnavailableHelp);
  expect(credentialUnavailableHelp("windows")).toBe(I18N.settings.credentialUnavailableHelp);
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
