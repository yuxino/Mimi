import { describe, expect, it } from "vitest";
import {
  buildProviderCredentials,
  buildAlibabaTranslationCredentials,
  deepLXEndpointIsValid,
  credentialEditorStateAfterDeleteRequest,
  credentialFieldsForProvider,
  emptyCredentialDraft,
  openAICompatibleModelIsValid,
} from "./providerCredentials";

describe("provider credential payloads", () => {
  it("keeps one-key providers on the compact credential shape", () => {
    const draft = emptyCredentialDraft();
    draft.apiKey = "  sk-test  ";
    expect(buildProviderCredentials("googleGeminiLive", draft)).toEqual({
      kind: "apiKey",
      apiKey: "sk-test",
    });
  });

  it("requires every provider-specific field", () => {
    expect(credentialFieldsForProvider("azureOpenAIRealtime")).toEqual([
      "endpoint",
      "deployment",
      "transcriptionDeployment",
      "apiKey",
    ]);
    const draft = emptyCredentialDraft();
    draft.endpoint = "https://mimi.openai.azure.com";
    draft.deployment = "translate";
    draft.transcriptionDeployment = "transcribe";
    expect(buildProviderCredentials("azureOpenAIRealtime", draft)).toBeNull();
    draft.apiKey = "secret";
    expect(buildProviderCredentials("azureOpenAIRealtime", draft)).toEqual({
      kind: "azureOpenAI",
      endpoint: "https://mimi.openai.azure.com",
      deployment: "translate",
      transcriptionDeployment: "transcribe",
      apiKey: "secret",
    });
  });

  it("describes multi-field Tencent and Baidu credentials", () => {
    expect(credentialFieldsForProvider("tencentCloud")).toEqual([
      "appId",
      "secretId",
      "secretKey",
    ]);
    expect(credentialFieldsForProvider("baiduTranslate")).toEqual([
      "appId",
      "appKey",
    ]);
  });

  it("clears every write-only field before a confirmed deletion", () => {
    const draft = emptyCredentialDraft();
    draft.apiKey = "new-api-secret";
    draft.appId = "123456";
    draft.secretId = "new-secret-id";
    draft.secretKey = "new-secret-key";

    const editing = { draft, editingSavedCredential: true };
    expect(credentialEditorStateAfterDeleteRequest(editing, false)).toBe(
      editing,
    );
    expect(credentialEditorStateAfterDeleteRequest(editing, true)).toEqual({
      draft: emptyCredentialDraft(),
      editingSavedCredential: false,
    });
  });
});

it("keeps DeepLX recognition credentials separate and makes its token optional", () => {
  const draft = { ...emptyCredentialDraft(), asrApiKey: " synthetic-asr ", endpoint: " https://example.com/translate " };
  expect(credentialFieldsForProvider("deepLX")).toEqual(["asrApiKey", "endpoint", "token"]);
  expect(buildProviderCredentials("deepLX", draft)).toEqual({ kind: "deepLX", asrApiKey: "synthetic-asr", endpoint: "https://example.com/translate", token: "" });
  expect(buildProviderCredentials("deepLX", { ...draft, asrApiKey: "" })).toBeNull();
  expect(buildProviderCredentials("deepLX", { ...draft, endpoint: "" })).toBeNull();
  expect(buildProviderCredentials("deepLX", { ...draft, token: " synthetic-token " })?.kind).toBe("deepLX");
});

 it("rejects unsafe or invalid DeepLX endpoints before credential submission", () => {
  for (const endpoint of ["not-a-url", "bad", "http://example.com", "https://user:password@example.com", "https://example.com?token=synthetic", "https://example.com#fragment", "https://example.com?", "https://example.com#", "https://example.com\n", "https://example.com/" + "x".repeat(2048)]) {
    expect(deepLXEndpointIsValid(endpoint), endpoint).toBe(false);
  }
  for (const endpoint of ["https://example.com", "https://example.com/api/translate", " http://localhost:1188/translate ", "http://127.0.0.1:1188", "http://[::1]:1188"]) {
    expect(deepLXEndpointIsValid(endpoint), endpoint).toBe(true);
  }
});


it("reuses a saved Alibaba key without sending a replacement over IPC", () => {
  const profile = { id: "ali", name: "Ali", provider: "alibabaCloud", credentialState: "present" } as const;
  const draft = { ...emptyCredentialDraft(), endpoint: " https://example.com " };
  expect(buildAlibabaTranslationCredentials(profile, draft, "deepLX")).toEqual({ kind: "alibabaTranslation", model: "", apiKey: "", textTranslation: "deepLX", endpoint: "https://example.com", token: "" });
  expect(buildAlibabaTranslationCredentials({ ...profile, credentialState: "missing" }, draft, "deepLX")).toBeNull();
  expect(buildAlibabaTranslationCredentials({ ...profile, provider: "openAIRealtime" }, draft, "deepLX")).toBeNull();
  expect(buildAlibabaTranslationCredentials(profile, emptyCredentialDraft(), "deepLX")).toBeNull();
  expect(buildAlibabaTranslationCredentials(profile, emptyCredentialDraft(), "followService")?.kind).toBe("alibabaTranslation");
  expect(buildAlibabaTranslationCredentials({ ...profile, textTranslation: "deepLX" }, emptyCredentialDraft(), "deepLX")?.kind).toBe("alibabaTranslation");
});

it("requires a DeepL key for a new destination and never sends an address", () => {
  const profile = { id: "ali", name: "Ali", provider: "alibabaCloud", credentialState: "present" } as const;
  const draft = { ...emptyCredentialDraft(), endpoint: "https://example.com/translate", token: " synthetic-deepl-key " };
  expect(buildAlibabaTranslationCredentials(profile, emptyCredentialDraft(), "deepL")).toBeNull();
  expect(buildAlibabaTranslationCredentials(profile, draft, "deepL")).toEqual({ kind: "alibabaTranslation", model: "", apiKey: "", textTranslation: "deepL", endpoint: "", token: "synthetic-deepl-key" });
  expect(buildAlibabaTranslationCredentials({ ...profile, textTranslation: "deepL" }, emptyCredentialDraft(), "deepL")).toEqual({ kind: "alibabaTranslation", model: "", apiKey: "", textTranslation: "deepL", endpoint: "", token: "" });
  expect(buildAlibabaTranslationCredentials({ ...profile, credentialState: "missing" }, draft, "deepL")).toBeNull();
  expect(buildAlibabaTranslationCredentials({ ...profile, credentialState: "missing" }, { ...draft, apiKey: " synthetic-asr " }, "deepL")).toEqual({ kind: "alibabaTranslation", model: "", apiKey: "synthetic-asr", textTranslation: "deepL", endpoint: "", token: "synthetic-deepl-key" });
});

it("requires an explicit model, endpoint and key for an OpenAI-compatible destination", () => {
  const profile = { id: "ali", name: "Ali", provider: "alibabaCloud", credentialState: "present" } as const;
  const draft = { ...emptyCredentialDraft(), endpoint: " https://dashscope.aliyuncs.com/compatible-mode/v1 ", token: " synthetic-translation-key ", model: " qwen-turbo " };
  expect(buildAlibabaTranslationCredentials(profile, draft, "openAICompatible")).toEqual({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1", token: "synthetic-translation-key", model: "qwen-turbo" });
  for (const field of ["endpoint", "token", "model"] as const) {
    expect(buildAlibabaTranslationCredentials(profile, { ...draft, [field]: "" }, "openAICompatible")).toBeNull();
  }
  expect(buildAlibabaTranslationCredentials({ ...profile, credentialState: "missing" }, draft, "openAICompatible")).toBeNull();
  expect(buildAlibabaTranslationCredentials({ ...profile, textTranslation: "openAICompatible" }, emptyCredentialDraft(), "openAICompatible")).toEqual({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "", token: "", model: "" });
});

it("requires a replacement OpenAI-compatible key with every changed address", () => {
  const profile = { id: "ali", name: "Ali", provider: "alibabaCloud", credentialState: "present", textTranslation: "openAICompatible" } as const;
  expect(buildAlibabaTranslationCredentials(profile, { ...emptyCredentialDraft(), endpoint: "https://new.example/v1" }, "openAICompatible")).toBeNull();
  expect(buildAlibabaTranslationCredentials(profile, { ...emptyCredentialDraft(), model: "new-model" }, "openAICompatible")).toMatchObject({ endpoint: "", token: "", model: "new-model" });
});

it("bounds OpenAI-compatible model names by bytes and rejects control characters", () => {
  for (const value of ["", "   ", "model\n", "model\u007f", "model\u0085", "x".repeat(257), "模".repeat(86)]) {
    expect(openAICompatibleModelIsValid(value), value).toBe(false);
  }
  for (const value of ["qwen-turbo", "provider/model-name", "x".repeat(256), "模".repeat(85)]) {
    expect(openAICompatibleModelIsValid(value), value).toBe(true);
  }
});
