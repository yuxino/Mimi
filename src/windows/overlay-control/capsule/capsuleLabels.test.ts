import { expect, it } from "vitest";
import { capsuleLabels } from "./capsuleLabels";

it("keeps Chinese error and language complete with intentional short labels", () => {
  expect(capsuleLabels({ sourceLanguage: "auto", targetLanguage: "zh" }, "error", "zh"))
    .toEqual({ source: "自动", target: "中文", phase: "错误" });
});

it("preserves original-only meaning and full phase labels in every UI language", () => {
  for (const language of ["en", "zh", "ja"] as const) {
    const settings = { sourceLanguage: "auto", targetLanguage: "zh" } as const;
    expect(capsuleLabels({ ...settings, targetLanguage: "original" }, null, language).target)
      .not.toBe(capsuleLabels(settings, null, language).target);
    expect(capsuleLabels(settings, "error", language).phase).not.toContain("…");
    expect(capsuleLabels(settings, "paused", language)).not.toHaveProperty("mode");
  }
});

it.each([
  ["zh", "法语", "繁体中文"],
  ["en", "French", "Traditional Chinese"],
  ["ja", "フランス語", "繁体中国語"],
] as const)("localizes extended languages with a script-aware %s fallback", (locale, source, target) => {
  expect(capsuleLabels({ sourceLanguage: "fr", targetLanguage: "zh_tw" }, null, locale))
    .toEqual({ source, target, phase: null });
});

it("does not claim automatic language detection for an arbitrary custom endpoint", () => {
  const settings = { sourceLanguage: "auto", targetLanguage: "original", activeProfileId: "custom", profiles: [{ id: "custom", name: "Custom", provider: "customOpenAIASR", credentialState: "present" }] } as const;
  expect(capsuleLabels({ ...settings, profiles: [...settings.profiles] }, null, "zh").source).toBe("服务默认");
});


it.each([["zh", "中英互译"], ["en", "Chinese ↔ English"], ["ja", "中国語 ↔ 英語"]] as const)("keeps %s bilingual reversal distinct from mixed recognition in the visible island", (language, label) => {
  const settings = { sourceLanguage: "zh_en", targetLanguage: "zh_en", activeProfileId: "test", profiles: [{ id: "test", name: "Volcano", provider: "volcanoEngine", credentialState: "present" }] } as const;
  expect(capsuleLabels({ ...settings, profiles: [...settings.profiles] }, null, language)).toEqual({ source: label, target: label, phase: null });
  expect(capsuleLabels({ ...settings, profiles: [{ ...settings.profiles[0], provider: "tencentCloud" }] }, null, language).source).not.toBe(label);
});
