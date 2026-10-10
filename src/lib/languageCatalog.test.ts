import { afterEach, expect, it } from "vitest";
import nativeModels from "../../shared/mimi-runtime/src/core/models.rs?raw";
import { setStoredUiLanguage } from "./i18n";
import {
  AUDIO3_RECOGNITION_LANGUAGE_CODES,
  SOURCE_LANGUAGE_CODES,
  TARGET_LANGUAGE_CODES,
  QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES,
  SOURCE_LANGUAGE_DISPLAY_NAMES,
  TARGET_LANGUAGE_DISPLAY_NAMES,
  sourceLanguageStatusDisplayName,
} from "./types";

afterEach(() => setStoredUiLanguage("en"));

it.each(["zh", "zh-TW", "en", "ja", "de", "fr", "ko", "th"] as const)("localizes every selectable wire code in %s", (locale) => {
  setStoredUiLanguage(locale);
  expect(Object.keys(SOURCE_LANGUAGE_DISPLAY_NAMES)).toEqual(["auto", ...SOURCE_LANGUAGE_CODES]);
  expect(Object.keys(TARGET_LANGUAGE_DISPLAY_NAMES)).toEqual(["original", ...TARGET_LANGUAGE_CODES]);
  for (const [code, name] of Object.entries(SOURCE_LANGUAGE_DISPLAY_NAMES)) {
    expect(name.trim()).not.toBe("");
    expect(name).not.toBe(code);
  }
  for (const [code, name] of Object.entries(TARGET_LANGUAGE_DISPLAY_NAMES)) {
    expect(name.trim()).not.toBe("");
    expect(name).not.toBe(code);
  }
});

it("updates expanded language labels in place and preserves stage/script distinctions", () => {
  setStoredUiLanguage("zh");
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.fr).toBe("法语");
  expect(TARGET_LANGUAGE_DISPLAY_NAMES.fa).toBe("波斯语");
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.zh).toBe("中文");
  expect(TARGET_LANGUAGE_DISPLAY_NAMES.zh).toBe("简体中文");
  expect(TARGET_LANGUAGE_DISPLAY_NAMES.zh_tw).toBe("繁体中文");
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.tl).toBe("菲律宾语");
  expect(TARGET_LANGUAGE_DISPLAY_NAMES.tl).toBe("塔加洛语");
  setStoredUiLanguage("en");
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.fr).toBe("French");
  expect(TARGET_LANGUAGE_DISPLAY_NAMES.zh_tw).toBe("Traditional Chinese");
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.tl).toBe("Filipino");
  expect(TARGET_LANGUAGE_DISPLAY_NAMES.tl).toBe("Tagalog");
  setStoredUiLanguage("ja");
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.fr).toBe("フランス語");
  expect(TARGET_LANGUAGE_DISPLAY_NAMES.fa).toBe("ペルシア語");
  expect(sourceLanguageStatusDisplayName("fr", null, "zh")).toBe("フランス語");
});


it("keeps the complete serializable registry separate from Audio3 and Qwen model support", () => {
  expect(SOURCE_LANGUAGE_CODES).toContain("uk");
  expect(SOURCE_LANGUAGE_CODES).toContain("yue");
  expect(TARGET_LANGUAGE_CODES).toContain("pt-BR");
  expect(TARGET_LANGUAGE_CODES).toContain("ast");
  expect(AUDIO3_RECOGNITION_LANGUAGE_CODES).toHaveLength(30);
  expect(QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES).toHaveLength(31);
  expect(AUDIO3_RECOGNITION_LANGUAGE_CODES).not.toContain("uk");
  expect(QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES).not.toContain("pt-BR");
  for (const [type, codes, sentinel] of [
    ["SourceLanguage", SOURCE_LANGUAGE_CODES, "auto"],
    ["TargetLanguage", TARGET_LANGUAGE_CODES, "original"],
  ] as const) {
    const block = nativeModels.split(`impl ${type} {`)[1].split("pub fn raw_value")[1].split("\n    }")[0];
    const nativeCodes = Array.from(block.matchAll(/=> "([^"]+)"/g), match => match[1]);
    expect(nativeCodes).toEqual([sentinel, ...codes]);
    expect(new Set(codes).size).toBe(codes.length);
  }
});

it("distinguishes regional variants and mixed Chinese-English from automatic recognition", () => {
  setStoredUiLanguage("zh");
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.zh_en).toBe("中英混合");
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.zh_en).not.toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.auto);
  expect(TARGET_LANGUAGE_DISPLAY_NAMES["pt-BR"]).toBe("巴西葡萄牙语");
  expect(TARGET_LANGUAGE_DISPLAY_NAMES["pt-PT"]).toBe("欧洲葡萄牙语");
  expect(SOURCE_LANGUAGE_DISPLAY_NAMES.zh_tw).toBe("繁体中文");
});
