import { afterEach, expect, it } from "vitest";
import { setStoredUiLanguage } from "../i18n";
import { audio3ErrorMessage, audio3ErrorRequiresConfiguration, audio3ErrorSummary } from "./audio3Errors";

afterEach(() => setStoredUiLanguage("en"));

it("separates service timeout, authentication, unknown failure and task phase", () => {
  setStoredUiLanguage("zh");
  expect(audio3ErrorMessage("audio3_error.recognition.timeout.CLIENT_ERROR")).toContain("服务超时");
  expect(audio3ErrorMessage("audio3_error.recognition.timeout.CLIENT_ERROR")).not.toContain("设备选错");
  expect(audio3ErrorMessage("audio3_error.setup.authentication.INVALID_API_KEY")).toContain("API 密钥");
  expect(audio3ErrorMessage("audio3_error.setup.task_failed.OTHER")).toContain("原因尚不明确");
  expect(audio3ErrorMessage("audio3_error.setup.request.CLIENT_ERROR")).toContain("启动语音识别时");
});

it("keeps recovery actions in English, Chinese and Japanese without exposing arbitrary codes", () => {
  for (const language of ["en", "zh", "ja"] as const) {
    setStoredUiLanguage(language);
    expect(audio3ErrorMessage("audio3_error.recognition.service.SERVER_ERROR")).toContain({ en: "Try again", zh: "重试", ja: "再試行" }[language]);
  }
  expect(audio3ErrorMessage("audio3_error.recognition.timeout.synthetic-private-code")).toBeNull();
  expect(audio3ErrorMessage("audio3_error.recognition.timeout.CLIENT_ERROR private text")).toBeNull();
});

it("explains unsupported language safely in all languages and requires configuration", () => {
  for (const language of ["en", "zh", "ja"] as const) {
    setStoredUiLanguage(language);
    const token = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
    expect(audio3ErrorMessage(token)).toContain({ en: "source language", zh: "源语言", ja: "入力言語" }[language]);
    expect(audio3ErrorRequiresConfiguration(token)).toBe(true);
    expect(audio3ErrorRequiresConfiguration("audio3_error.setup.authentication.INVALID_API_KEY")).toBe(true);
    expect(audio3ErrorRequiresConfiguration("audio3_error.setup.request.CLIENT_ERROR")).toBe(true);
    expect(audio3ErrorRequiresConfiguration(token + ".private")).toBe(false);
    expect(audio3ErrorMessage(token + ".private")).toBeNull();
    expect(audio3ErrorRequiresConfiguration("audio3_error.setup.timeout.CLIENT_ERROR")).toBe(false);
    expect(audio3ErrorRequiresConfiguration("audio3_error.recognition.service.SERVER_ERROR")).toBe(false);
  }
});

it("keeps runtime overload, timeout and generic request failure retryable", () => {
  for (const language of ["en", "zh", "ja"] as const) {
    setStoredUiLanguage(language);
    for (const reason of ["local_overload.LOCAL_ASR_OVERLOADED", "local_timeout.LOCAL_ASR_TIMEOUT"]) {
      const token = `audio3_error.recognition.${reason}`;
      expect(audio3ErrorMessage(token)).toContain({ en: "local speech recognition", zh: "本地语音识别", ja: "ローカル音声認識" }[language]);
      expect(audio3ErrorRequiresConfiguration(token)).toBe(false);
      expect(audio3ErrorMessage(token + " private")).toBeNull();
    }
    expect(audio3ErrorRequiresConfiguration("audio3_error.recognition.request.CLIENT_ERROR")).toBe(false);
  }
});

it.each([
  ["en", "The speech service does not support this source language."],
  ["zh", "语音识别服务不支持所选源语言。"],
  ["ja", "音声認識サービスがこの入力言語に対応していません。"],
] as const)("keeps a separate short cause with the same safe allowlist in %s", (language, expected) => {
  setStoredUiLanguage(language);
  const token = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
  expect(audio3ErrorSummary(token)).toBe(expected);
  expect(audio3ErrorMessage(token)).not.toBe(expected);
  expect(audio3ErrorSummary(token + " private provider text")).toBeNull();
  expect(audio3ErrorSummary("audio3_error.setup.unsupported_language.PRIVATE_CODE")).toBeNull();
});
