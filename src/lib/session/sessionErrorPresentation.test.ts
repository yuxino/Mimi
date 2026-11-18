import { expect, it } from "vitest";
import { localizedSessionErrorSummary } from "./sessionErrorPresentation";

it.each([
  ["Could not connect to speech recognition. Check the address and network.", "Could not connect to speech recognition."],
  ["无法连接语音识别服务。请检查地址和网络。", "无法连接语音识别服务。"],
  ["音声認識サービスに接続できません。設定を確認してください。", "音声認識サービスに接続できません。"],
  ["Cannot read the local .env. Check its permissions.", "Cannot read the local .env."],
  ["字幕启动失败", "字幕启动失败"],
])("retains the first complete reason from an already localized message", (message, expected) => {
  expect(localizedSessionErrorSummary(message)).toBe(expected);
});
