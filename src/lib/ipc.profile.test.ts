import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { profileSelect, profileUpdate, testProfileConnection } from "./ipc";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue({}) }));
beforeEach(() => vi.mocked(invoke).mockClear());
it("omits language patches on unrelated profile updates", async () => {
  await profileUpdate("p", "Name", { textTranslationName: { route: "chatMock", name: "Local" } });
  expect(invoke).toHaveBeenCalledExactlyOnceWith("profile_update", { profileId: "p", name: "Name", textTranslationName: { route: "chatMock", name: "Local" } });
});
it.each([null, [], ["en", "fr"]] as const)("preserves the explicit language patch %j", async languages => {
  await profileUpdate("p", undefined, { customSpeechSourceLanguages: languages === null ? null : [...languages] });
  expect(invoke).toHaveBeenCalledExactlyOnceWith("profile_update", { profileId: "p", name: undefined, customSpeechLanguagesPatch: { languages } });
});

it("keeps recognition names and language declarations as independent IPC patches", async () => {
  await profileUpdate("p", undefined, { speechRecognitionName: "Recognizer" });
  expect(invoke).toHaveBeenLastCalledWith("profile_update", { profileId: "p", name: undefined, speechRecognitionName: "Recognizer" });
  await profileUpdate("p", undefined, { speechRecognitionName: "Renamed", customSpeechSourceLanguages: ["en"] });
  expect(invoke).toHaveBeenLastCalledWith("profile_update", { profileId: "p", name: undefined, speechRecognitionName: "Renamed", customSpeechLanguagesPatch: { languages: ["en"] } });
});


it("sends explicit Apple language selection in the same profile command", async () => {
  await profileSelect("apple", "ja");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("profile_select", { profileId: "apple", sourceLanguage: "ja" });
});

it("checks a displayed Apple language without submitting a settings mutation", async () => {
  await testProfileConnection("apple", "speech", undefined, "ja");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("profile_test_connection", { profileId: "apple", stage: "speech", sourceLanguage: "ja" });
});
