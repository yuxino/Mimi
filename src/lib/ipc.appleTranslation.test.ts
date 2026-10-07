import { afterEach, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn().mockResolvedValue("installed"));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); invoke.mockClear(); });

it("reads native support and the exact selected pair without requesting a download", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const ipc = await import("./ipc");
  await ipc.getAppleTranslationSupport();
  await ipc.getAppleTranslationStatus("ja", "zh_tw");
  expect(invoke.mock.calls).toEqual([
    ["get_apple_translation_support"],
    ["get_apple_translation_status", { sourceLanguage: "ja", targetLanguage: "zh_tw" }],
  ]);
});

it("prepares only the explicit pair with the effective interface language", async () => {
  const { setStoredUiLanguage } = await import("./i18n");
  setStoredUiLanguage("ja");
  const { prepareAppleTranslationLanguages } = await import("./ipc");
  await prepareAppleTranslationLanguages("en", "zh");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("prepare_apple_translation_languages", { sourceLanguage: "en", targetLanguage: "zh", uiLanguage: "ja" });
  setStoredUiLanguage("system");
});

it("does not advertise native translation in a plain browser", async () => {
  const ipc = await import("./ipc");
  expect(await ipc.getAppleTranslationSupport()).toEqual({ available: false, sourceLanguages: [], targetLanguages: [] });
  expect(await ipc.getAppleTranslationStatus("en", "zh")).toBe("unavailable");
  expect(invoke).not.toHaveBeenCalled();
});
