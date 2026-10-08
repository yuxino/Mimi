import { afterEach, expect, it } from "vitest";
import { I18N, setStoredUiLanguage } from "./i18n";
import { LOCAL_MODEL_COPY } from "./localModelI18n";
import { speechLanguageGuidance, targetLanguageOptionLabel } from "./speechLanguageGuidance";
import type { ServiceProvider } from "./types";
const settings = (provider: ServiceProvider) => ({ profiles: [{ id: "p", name: "P", provider, credentialState: "missing" as const }], activeProfileId: "p", targetLanguage: "original" as const });
afterEach(() => setStoredUiLanguage("system"));
it("explains Apple's explicit source requirement on shared settings, tray and overlay controls", () => {
  const base = settings("alibabaCloud");
  const profile = { ...base.profiles[0], textTranslation: "apple" as const };
  expect(speechLanguageGuidance({ ...base, profiles: [profile], targetLanguage: "zh" }).help).toContain(I18N.settings.appleTranslationChooseSource);
  expect(speechLanguageGuidance({ ...base, profiles: [profile] }).help).not.toContain(I18N.settings.appleTranslationChooseSource);
});
it.each(["zh", "en", "ja"] as const)("separates automatic, hint, explicit and unknown semantics in %s", locale => {
  setStoredUiLanguage(locale);
  expect(speechLanguageGuidance(settings("openAIRealtime")).help).toBe(I18N.settings.recognitionAutomaticHelp);
  expect(speechLanguageGuidance(settings("googleGeminiLive")).help).toBe(I18N.settings.recognitionGeminiAutomaticHelp);
  expect(speechLanguageGuidance(settings("xAIRealtime")).help).toContain(I18N.settings.recognitionHintHelp);
  expect(speechLanguageGuidance(settings("localSpeech")).help).toBe(LOCAL_MODEL_COPY.sourceHelp);
  const explicit = speechLanguageGuidance(settings("tencentCloud"));
  expect(explicit.help).toContain(I18N.settings.recognitionExplicitHelp);
  expect(explicit.help).not.toContain(I18N.settings.recognitionHintHelp);
  for (const provider of ["customDashScopeASR", "customOpenAIASR"] as const) {
    const unknown = speechLanguageGuidance(settings(provider));
    expect(unknown.notice).toBe(I18N.settings.recognitionCustomNotice);
    expect(unknown.optionLabel("auto")).toBe(I18N.settings.recognitionServiceDefault);
    expect(unknown.help).toContain(I18N.settings.recognitionCustomHelp);
    expect(unknown.help).toContain(provider === "customDashScopeASR" ? "language_hints" : "transcription.languages");
    expect(unknown.help).not.toContain(I18N.settings.recognitionAutomaticHelp);
  }
});

it("labels explicit declarations without claiming discovered or verified support", () => {
  const unknown = settings("customOpenAIASR");
  const declared = { ...unknown, profiles: [{ ...unknown.profiles[0], customSpeechSourceLanguages: ["fr" as const] }] };
  expect(speechLanguageGuidance(declared).notice).toBe(I18N.settings.recognitionDeclaredNotice);
  expect(speechLanguageGuidance(declared).optionLabel("auto")).toBe(I18N.settings.recognitionServiceDefault);
});

it.each(["zh", "en", "ja"] as const)("keeps Volcano reversal distinct from Tencent mixed recognition in %s", locale => {
  setStoredUiLanguage(locale);
  const volcano = settings("volcanoEngine");
  expect(speechLanguageGuidance(volcano).optionLabel("zh_en")).toBe(I18N.settings.recognitionVolcanoBilingual);
  expect(targetLanguageOptionLabel(volcano, "zh_en")).toBe(I18N.settings.recognitionVolcanoBilingual);
  expect(speechLanguageGuidance(volcano).help).toBe(I18N.settings.recognitionVolcanoHelp);
  expect(speechLanguageGuidance(settings("tencentCloud")).optionLabel("zh_en")).not.toBe(I18N.settings.recognitionVolcanoBilingual);
});
