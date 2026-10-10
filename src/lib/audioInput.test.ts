import { afterEach, expect, it } from "vitest";
import { audioInputErrorMessage, audioInputLabel } from "./audioInput";
import { setStoredUiLanguage, I18N } from "./i18n";

afterEach(() => setStoredUiLanguage("en"));
it.each(["en", "zh", "ja"] as const)("localizes microphone failures with recovery actions in %s", language => {
  setStoredUiLanguage(language);
  const missing = audioInputErrorMessage("No default microphone is available.");
  const denied = audioInputErrorMessage("Microphone capture permission was denied.");
  const failed = audioInputErrorMessage("Microphone capture could not be started.");
  expect(missing).toContain({ en: "default input", zh: "默认输入", ja: "既定の入力" }[language]);
  expect(denied).toContain({ en: "privacy settings", zh: "隐私设置", ja: "プライバシー設定" }[language]);
  expect(failed).toContain({ en: "try again", zh: "重试", ja: "再試行" }[language]);
  expect(audioInputErrorMessage("Audio capture stopped unexpectedly.")).toContain({ en: "selected input", zh: "所选输入", ja: "選択した入力" }[language]);
});
it("matches only complete safe labels and does not reinterpret provider text", () => {
  expect(audioInputErrorMessage("No default microphone is available. synthetic-private-detail")).toBeNull();
  expect(audioInputErrorMessage("unrelated-provider-failure")).toBeNull();
});

it("identifies the selected app and keeps the microphone independent", () => {
  const target = { kind: "application" as const, id: "test.player", name: "Player" };
  expect(audioInputLabel("system", target)).toBe("Player");
  expect(audioInputLabel("both", target)).toBe(`Player + ${I18N.settings.audioInputMicrophone}`);
  expect(audioInputLabel("microphone", target)).toBe(I18N.settings.audioInputMicrophone);
});

it.each(["zh", "zh-TW", "en", "ja", "de", "ko", "fr", "th"] as const)("keeps system permission denial distinct from microphone or provider errors in %s", language => {
  setStoredUiLanguage(language);
  const message = audioInputErrorMessage("System audio capture permission was denied.");
  expect(message).toBeTruthy();
  expect(message).toContain("Mimi");
  expect(message).toContain("→");
  expect(message).not.toBe(audioInputErrorMessage("Microphone capture permission was denied."));
  expect(audioInputErrorMessage("System audio capture permission was denied. private-native-detail")).toBeNull();
});
