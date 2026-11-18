import { expect, it } from "vitest";
import { compatiblePresetForEndpoint } from "./textTranslationPresets";

it("recognizes a preset with a full completion path while preserving its API prefix", () => {
  expect(compatiblePresetForEndpoint("https://generativelanguage.googleapis.com/v1beta/openai/chat/completions/")?.id).toBe("gemini");
  expect(compatiblePresetForEndpoint("https://generativelanguage.googleapis.com/v1beta")?.id).toBeUndefined();
  expect(compatiblePresetForEndpoint("http://127.0.0.1:11434/v1")?.id).toBe("ollama");
});

it("keeps unknown and credential-bearing URLs custom", () => {
  for (const endpoint of ["", "invalid", "https://api.openai.com.evil.example/v1", "https://user:secret@api.openai.com/v1", "https://api.openai.com/v1?token=secret", "https://api.openai.com/v1#fragment"]) {
    expect(compatiblePresetForEndpoint(endpoint)).toBeUndefined();
  }
});
