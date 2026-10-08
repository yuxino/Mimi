import { describe, expect, it } from "vitest";
import {
  buildProviderCredentials,
  buildProviderProbeCredentials,
  buildCustomSpeechCredentials,
  customSpeechEndpointIsValid,
  buildAlibabaTranslationCredentials,
  deepLXEndpointIsValid,
  credentialEditorStateAfterDeleteRequest,
  credentialFieldsForProvider,
  emptyCredentialDraft,
  openAICompatibleModelIsValid,
} from "./providerCredentials";

it("switches to Apple without leaking old text fields or requiring a missing cloud text key", () => {
  const profile = { id: "ali", name: "Alibaba", provider: "alibabaCloud", credentialState: "missing", speechCredentialState: "present", textCredentialState: "missing", textTranslation: "deepL" } as const;
  const draft = { ...emptyCredentialDraft(), endpoint: "https://example.com", token: "synthetic-old-text-token", model: "old-model" };
  expect(buildAlibabaTranslationCredentials(profile, draft, "apple")).toEqual({ kind: "alibabaTranslation", apiKey: "", textTranslation: "apple", endpoint: "", token: "", model: "" });
  expect(buildAlibabaTranslationCredentials(profile, { ...draft, apiKey: "new-speech-key" }, "apple")).toMatchObject({ apiKey: "new-speech-key", token: "" });
  expect(buildAlibabaTranslationCredentials({ ...profile, provider: "appleSpeech" }, { ...draft, apiKey: "never-text" }, "apple")).toMatchObject({ apiKey: "", token: "" });
});

it("has no Apple speech credentials while preserving independent optional-auth text translation", () => {
  const profile = { id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "missing", textCredentialState: "missing" } as const;
  expect(credentialFieldsForProvider("appleSpeech")).toEqual([]);
  expect(buildProviderCredentials("appleSpeech", emptyCredentialDraft())).toBeNull();
  expect(buildCustomSpeechCredentials(profile, { endpoint: "ws://127.0.0.1:1", model: "model", apiKey: "synthetic" })).toBeNull();
  expect(buildAlibabaTranslationCredentials(profile, { ...emptyCredentialDraft(), endpoint: "http://127.0.0.1:18080/v1", model: "synthetic-model", apiKey: "never-send-asr-key" }, "openAICompatible"))
    .toMatchObject({ kind: "alibabaTranslation", apiKey: "", endpoint: "http://127.0.0.1:18080/v1", token: "", model: "synthetic-model" });
});

it("restricts custom speech to secure full WebSocket addresses or loopback", () => {
  for (const endpoint of ["https://example.com", "ws://example.com/asr", "wss://user:password@example.com", "wss://example.com?intent=transcription", "wss://example.com#", "wss://example.com\n"]) expect(customSpeechEndpointIsValid(endpoint), endpoint).toBe(false);
  for (const endpoint of ["wss://example.com/asr", "ws://localhost:1888/asr", "ws://127.0.0.1:1888", "ws://[::1]:1888"]) expect(customSpeechEndpointIsValid(endpoint), endpoint).toBe(true);
});
it("never reuses a saved recognition key at a different address", () => {
  const profile = { id: "asr", name: "ASR", provider: "customOpenAIASR", credentialState: "missing", speechCredentialState: "present" } as const;
  expect(buildCustomSpeechCredentials(profile, { endpoint: "wss://new.example", model: "", apiKey: "" })).toBeNull();
  expect(buildCustomSpeechCredentials(profile, { endpoint: "", model: "new-model", apiKey: "" })).toEqual({ kind: "customSpeech", endpoint: "", model: "new-model", apiKey: "" });
  expect(buildCustomSpeechCredentials({ ...profile, speechCredentialState: "missing" }, { endpoint: "", model: "new-model", apiKey: "synthetic-key" })).toBeNull();
});
it("uses the independent text state and never sends a custom ASR key in text-only saves", () => {
  const profile = { id: "asr", name: "ASR", provider: "customDashScopeASR", credentialState: "missing", speechCredentialState: "present", textCredentialState: "missing", textTranslation: "deepL" } as const;
  expect(buildAlibabaTranslationCredentials(profile, emptyCredentialDraft(), "deepL")).toBeNull();
  expect(buildAlibabaTranslationCredentials(profile, { ...emptyCredentialDraft(), apiKey: "synthetic-asr-must-not-send", token: "synthetic-text-key" }, "deepL")).toMatchObject({ apiKey: "", token: "synthetic-text-key" });
  expect(buildAlibabaTranslationCredentials({ ...profile, speechCredentialState: "missing" }, { ...emptyCredentialDraft(), apiKey: "synthetic-asr", token: "synthetic-text-key" }, "deepL")).toMatchObject({ apiKey: "", token: "synthetic-text-key" });
});

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

describe("temporary provider check payloads", () => {
  it.each(["alibabaCloud", "openAIRealtime", "googleGeminiLive", "volcanoEngine", "xAIRealtime"] as const)(
    "keeps an empty %s saved key in native storage and prefers an explicit replacement", provider => {
      const draft = emptyCredentialDraft();
      expect(buildProviderProbeCredentials(provider, draft)).toBeNull();
      expect(buildProviderProbeCredentials(provider, draft, ["apiKey"])).toEqual({ kind: "apiKey", apiKey: "" });
      expect(buildProviderCredentials(provider, draft)).toBeNull();
      expect(buildProviderProbeCredentials(provider, { ...draft, apiKey: "  synthetic-new-key  " }, ["apiKey"]))
        .toEqual({ kind: "apiKey", apiKey: "synthetic-new-key" });
      expect(draft).toEqual(emptyCredentialDraft());
    },
  );

  it("requires Azure endpoint and deployment metadata while allowing only its known saved key to remain blank", () => {
    const draft = { ...emptyCredentialDraft(), endpoint: " https://synthetic.openai.azure.com ", deployment: " changed-translation ", transcriptionDeployment: " saved-transcription " };
    expect(buildProviderProbeCredentials("azureOpenAIRealtime", draft, ["apiKey"])).toEqual({
      kind: "azureOpenAI", endpoint: "https://synthetic.openai.azure.com", deployment: "changed-translation",
      transcriptionDeployment: "saved-transcription", apiKey: "",
    });
    for (const field of ["endpoint", "deployment", "transcriptionDeployment"] as const) {
      expect(buildProviderProbeCredentials("azureOpenAIRealtime", { ...draft, [field]: "" }, ["apiKey"])).toBeNull();
    }
    expect(buildProviderProbeCredentials("azureOpenAIRealtime", draft, ["appKey"])).toBeNull();
    expect(buildProviderCredentials("azureOpenAIRealtime", draft)).toBeNull();
  });

  it("builds partial Tencent checks from the exact saved secret slots without changing normal saves", () => {
    const draft = { ...emptyCredentialDraft(), appId: " 123456 ", secretId: " synthetic-new-id " };
    expect(buildProviderProbeCredentials("tencentCloud", draft, ["secretKey"])).toEqual({
      kind: "tencentCloud", appId: "123456", secretId: "synthetic-new-id", secretKey: "",
    });
    expect(buildProviderProbeCredentials("tencentCloud", draft, ["secretId", "apiKey"])).toBeNull();
    expect(buildProviderProbeCredentials("tencentCloud", { ...draft, appId: "" }, ["secretId", "secretKey"])).toBeNull();
    expect(buildProviderProbeCredentials("tencentCloud", { ...draft, secretId: "" }, ["secretId", "secretKey"]))
      .toEqual({ kind: "tencentCloud", appId: "123456", secretId: "", secretKey: "" });
    expect(buildProviderCredentials("tencentCloud", draft)).toBeNull();
  });

  it("keeps Baidu app identity mandatory and does not borrow another provider's saved key marker", () => {
    const draft = { ...emptyCredentialDraft(), appId: " 123456 " };
    expect(buildProviderProbeCredentials("baiduTranslate", draft, ["appKey"])).toEqual({
      kind: "baiduTranslate", appId: "123456", appKey: "",
    });
    expect(buildProviderProbeCredentials("baiduTranslate", draft, ["apiKey"])).toBeNull();
    expect(buildProviderProbeCredentials("baiduTranslate", emptyCredentialDraft(), ["appKey"])).toBeNull();
    expect(buildProviderProbeCredentials("baiduTranslate", { ...draft, appKey: " synthetic-new-app-key " }, ["appKey"]))
      .toEqual({ kind: "baiduTranslate", appId: "123456", appKey: "synthetic-new-app-key" });
    expect(buildProviderCredentials("baiduTranslate", draft)).toBeNull();
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

it("requires a model and endpoint but permits an unauthenticated OpenAI-compatible destination", () => {
  const profile = { id: "ali", name: "Ali", provider: "alibabaCloud", credentialState: "present" } as const;
  const draft = { ...emptyCredentialDraft(), endpoint: " https://dashscope.aliyuncs.com/compatible-mode/v1 ", token: " synthetic-translation-key ", model: " qwen-turbo " };
  expect(buildAlibabaTranslationCredentials(profile, draft, "openAICompatible")).toEqual({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1", token: "synthetic-translation-key", model: "qwen-turbo" });
  for (const field of ["endpoint", "model"] as const) {
    expect(buildAlibabaTranslationCredentials(profile, { ...draft, [field]: "" }, "openAICompatible")).toBeNull();
  }
  expect(buildAlibabaTranslationCredentials(profile, { ...draft, token: "" }, "openAICompatible")).toMatchObject({ apiKey: "", token: "" });
  expect(buildAlibabaTranslationCredentials({ ...profile, credentialState: "missing" }, draft, "openAICompatible")).toBeNull();
  expect(buildAlibabaTranslationCredentials({ ...profile, textTranslation: "openAICompatible" }, emptyCredentialDraft(), "openAICompatible")).toEqual({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "", token: "", model: "" });
});

it("sends an empty optional key for a changed address and preserves native reuse for model-only edits", () => {
  const profile = { id: "ali", name: "Ali", provider: "alibabaCloud", credentialState: "present", textTranslation: "openAICompatible" } as const;
  expect(buildAlibabaTranslationCredentials(profile, { ...emptyCredentialDraft(), endpoint: "http://localhost:8080/v1" }, "openAICompatible")).toEqual({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "http://localhost:8080/v1", token: "", model: "" });
  expect(buildAlibabaTranslationCredentials(profile, { ...emptyCredentialDraft(), model: "new-model" }, "openAICompatible")).toEqual({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "", token: "", model: "new-model" });
});

it("distinguishes explicit translation-key removal from blank saved fields", () => {
  const profile = { id: "ali", name: "Ali", provider: "alibabaCloud", credentialState: "present", textTranslation: "openAICompatible" } as const;
  const draft = { ...emptyCredentialDraft(), token: "synthetic-replacement" };
  expect(buildAlibabaTranslationCredentials(profile, draft, "openAICompatible", true)).toEqual({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "", token: "", model: "", clearToken: true });
  expect(buildAlibabaTranslationCredentials(profile, draft, "deepL", true)).toMatchObject({ token: "synthetic-replacement" });
  expect(buildAlibabaTranslationCredentials(profile, draft, "deepL", true)).not.toHaveProperty("clearToken");
});

it.each(["customDashScopeASR", "customOpenAIASR"] as const)("does not borrow the %s recognition key for a keyless ChatMock destination", (provider) => {
  const profile = { id: "asr", name: "ASR", provider, credentialState: "missing", speechCredentialState: "present", textCredentialState: "missing" } as const;
  const draft = { ...emptyCredentialDraft(), endpoint: "http://localhost:8080/v1", model: "synthetic-model", apiKey: "synthetic-asr-must-not-send" };
  expect(buildAlibabaTranslationCredentials(profile, draft, "openAICompatible")).toEqual({ kind: "alibabaTranslation", apiKey: "", textTranslation: "openAICompatible", endpoint: "http://localhost:8080/v1", token: "", model: "synthetic-model" });
  expect(buildCustomSpeechCredentials({ ...profile, speechCredentialState: "missing" }, { endpoint: "wss://synthetic.example/asr", model: "synthetic-asr", apiKey: "" })).toBeNull();
});

it("bounds OpenAI-compatible model names by bytes and rejects control characters", () => {
  for (const value of ["", "   ", "model\n", "model\u007f", "model\u0085", "x".repeat(257), "模".repeat(86)]) {
    expect(openAICompatibleModelIsValid(value), value).toBe(false);
  }
  for (const value of ["qwen-turbo", "provider/model-name", "x".repeat(256), "模".repeat(85)]) {
    expect(openAICompatibleModelIsValid(value), value).toBe(true);
  }
});


it.each(["customDashScopeASR", "customOpenAIASR"] as const)("allows keyless literal-loopback %s services only", provider => {
  const profile = { id: "local", name: "Own service", provider, credentialState: "missing" as const };
  for (const endpoint of ["ws://127.0.0.1:8080/realtime", "ws://[::1]:8080/realtime", "wss://localhost/realtime"]) {
    const draft = { ...emptyCredentialDraft(), endpoint, model: "own-model" };
    expect(buildCustomSpeechCredentials(profile, draft)).toMatchObject({ apiKey: "", endpoint, model: "own-model" });
    expect(buildProviderCredentials(provider, draft)).toMatchObject({ apiKey: "" });
  }
  for (const endpoint of ["wss://speech.example/realtime", "wss://localhost.example/realtime", "ws://127.0.0.1:8080/realtime?token=secret"]) {
    expect(buildCustomSpeechCredentials(profile, { endpoint, model: "own-model", apiKey: "" })).toBeNull();
  }
});
