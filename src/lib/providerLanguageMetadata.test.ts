import audio3 from "../../src-tauri/src/core/protocols/audio3.rs?raw";
import rust from "../../src-tauri/src/core/protocols/qwen_mt.rs?raw";
import { expect, it } from "vitest";
import { capabilitiesForProvider, SERVICE_PROVIDERS } from "./providerCapabilities";
import { AUDIO3_LITE_PIPELINE_SOURCE_CODES, AUDIO3_RECOGNITION_LANGUAGE_CODES, languageMetadataForProfile, providerLanguageDisplayCode, QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES } from "./providerLanguageMetadata";
import type { ServiceProfile } from "./types";

const profile: ServiceProfile = { id: "synthetic", name: "Alibaba", provider: "alibabaCloud", credentialState: "missing" };

it("distinguishes translated intersection choices from complete Audio3/Lite model catalogs", () => {
  const metadata = languageMetadataForProfile(profile);
  expect(metadata.appSelectable.sourceCodes).toEqual(["auto", ...AUDIO3_LITE_PIPELINE_SOURCE_CODES]);
  expect(metadata.appSelectable.targetCodes).toEqual(["original", ...QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES]);
  expect(languageMetadataForProfile(profile, "original").appSelectable.sourceCodes).toEqual(["auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES]);
  expect(metadata.providerAvailable.recognition.languageCodes).toHaveLength(30);
  expect(metadata.providerAvailable.translation.languageCodes).toHaveLength(31);
  expect(new Set(AUDIO3_RECOGNITION_LANGUAGE_CODES).size).toBe(30);
  expect(new Set(QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES).size).toBe(31);
  expect(metadata.providerAvailable.recognition.automaticDetection).toBe("supported");
  expect(metadata.providerAvailable.translation.model).toBe("qwen-mt-lite");
});

it("restricts the pipeline to its intersection without claiming recognition of text-only languages", () => {
  expect(AUDIO3_LITE_PIPELINE_SOURCE_CODES).toHaveLength(24);
  for (const code of ["no", "ro", "el", "bg", "hr", "sk"]) {
    expect(AUDIO3_RECOGNITION_LANGUAGE_CODES).toContain(code);
    expect(AUDIO3_LITE_PIPELINE_SOURCE_CODES).not.toContain(code);
  }
  for (const code of ["he", "ur", "bn", "tr", "km", "fa", "zh_tw"]) {
    expect(QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES).toContain(code);
    expect(AUDIO3_LITE_PIPELINE_SOURCE_CODES).not.toContain(code);
  }
  expect(AUDIO3_LITE_PIPELINE_SOURCE_CODES).not.toContain("auto");
  expect(QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES).not.toContain("original");
});

it("keeps display evidence aligned with the actual Rust model and exact Lite table", () => {
  expect(audio3).toMatch(/pub const MODEL:\s*&'static str\s*=\s*"qwen-audio-3\.0-asr-flash-streaming";/);
  expect(rust).toMatch(/pub const REALTIME_MT_MODEL:\s*QwenMTModel\s*=\s*QwenMTModel::Lite;/);
  const table = rust.match(/pub const QWEN_MT_LITE_LANGUAGE_CODES:[\s\S]*?=\s*&\[([\s\S]*?)\];/)?.[1];
  expect(table).toBeDefined();
  expect(Array.from(table!.matchAll(/"([a-z_]+)"/g), (match) => match[1])).toEqual(QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES);
});

it("never borrows Lite's target list for independent official or custom destinations", () => {
  for (const [textTranslation, limitation] of [["deepL", "resourceQueryRequired"], ["deepLX", "customUnknown"], ["openAICompatible", "customUnknown"]] as const) {
    const metadata = languageMetadataForProfile({ ...profile, textTranslation });
    expect(metadata.providerAvailable.recognition.languageCodes).toEqual(AUDIO3_RECOGNITION_LANGUAGE_CODES);
    expect(metadata.providerAvailable.translation.languageCodes).toBeNull();
    expect(metadata.providerAvailable.explicitPipelineSourceCodes).toBeNull();
    expect(metadata.providerAvailable.limitation).toBe(limitation);
    expect(metadata.appSelectable.sourceCodes).toEqual(["auto", "ja", "en", "ko", "zh"]);
    expect(metadata.appSelectable.targetCodes).not.toContain("fr");
  }
  const legacy = languageMetadataForProfile({ ...profile, provider: "deepLX" });
  expect(legacy.providerAvailable.limitation).toBe("customUnknown");
  expect(legacy.appSelectable.targetCodes).not.toContain("original");
  expect(languageMetadataForProfile({ ...profile, provider: "deepLX", textTranslation: "followService" }).providerAvailable.translation.model).toBe("qwen-mt-lite");
});

it("keeps other providers' selectors and unknown full ranges separate", () => {
  for (const provider of SERVICE_PROVIDERS.filter((provider) => provider !== "alibabaCloud")) {
    const metadata = languageMetadataForProfile({ ...profile, provider, textTranslation: "deepL" });
    expect(metadata.appSelectable.sourceCodes).toEqual(capabilitiesForProvider(provider).sourceLanguages);
    expect(metadata.appSelectable.targetCodes).toEqual(capabilitiesForProvider(provider).targetLanguages);
    expect(metadata.providerAvailable.recognition.languageCodes).toBeNull();
    expect(metadata.providerAvailable.translation.languageCodes).toBeNull();
    expect(metadata.providerAvailable.limitation).toBe("unverified");
  }
  expect(languageMetadataForProfile({ ...profile, provider: "baiduTranslate" }).providerAvailable.recognition.automaticDetection).toBe("unsupported");
  expect(languageMetadataForProfile({ ...profile, provider: "tencentCloud" }).providerAvailable.recognition.automaticDetection).toBe("unsupported");
  expect(languageMetadataForProfile({ ...profile, provider: "volcanoEngine" }).providerAvailable.recognition.automaticDetection).toBe("unverified");
});

it("preserves Chinese scripts and the upstream Tagalog code for display", () => {
  expect(providerLanguageDisplayCode("zh", "recognition")).toBe("zh");
  expect(providerLanguageDisplayCode("zh", "translation")).toBe("zh-Hans");
  expect(providerLanguageDisplayCode("zh_tw", "translation")).toBe("zh-Hant");
  expect(providerLanguageDisplayCode("tl", "recognition")).toBe("fil");
  expect(providerLanguageDisplayCode("tl", "translation")).toBe("tl");
});
