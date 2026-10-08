import type {
  ProviderCredentialsInput,
  ServiceProfile,
  TextTranslation,
  ServiceProvider,
} from "./types";
import { isChatCompletionsTranslation, isCustomSpeechProvider, isStandaloneAsrProvider } from "./providerCapabilities";

export type CredentialFieldName =
  | "asrApiKey"
  | "token"
  | "apiKey"
  | "endpoint"
  | "model"
  | "deployment"
  | "transcriptionDeployment"
  | "appId"
  | "secretId"
  | "secretKey"
  | "appKey";

export type CredentialDraft = Record<CredentialFieldName, string> & { model: string };

interface CredentialEditorLocalState {
  draft: CredentialDraft;
  editingSavedCredential: boolean;
}

export function emptyCredentialDraft(): CredentialDraft {
  return {
    asrApiKey: "",
    token: "",
    apiKey: "",
    endpoint: "",
    model: "",
    deployment: "",
    transcriptionDeployment: "",
    appId: "",
    secretId: "",
    secretKey: "",
    appKey: "",
  };
}

/** Clears write-only fields before a confirmed credential deletion starts. */
export function credentialEditorStateAfterDeleteRequest(
  current: CredentialEditorLocalState,
  confirmingDelete: boolean,
): CredentialEditorLocalState {
  if (!confirmingDelete) return current;
  return {
    draft: emptyCredentialDraft(),
    editingSavedCredential: false,
  };
}

export function credentialFieldsForProvider(
  provider: ServiceProvider,
): readonly CredentialFieldName[] {
  switch (provider) {
    case "appleSpeech":
      return [];
    case "customDashScopeASR":
    case "customOpenAIASR":
      return ["endpoint", "model", "apiKey"];
    case "deepLX":
      return ["asrApiKey", "endpoint", "token"];
    case "azureOpenAIRealtime":
      return [
        "endpoint",
        "deployment",
        "transcriptionDeployment",
        "apiKey",
      ];
    case "tencentCloud":
      return ["appId", "secretId", "secretKey"];
    case "baiduTranslate":
      return ["appId", "appKey"];
    default:
      return ["apiKey"];
  }
}

export function buildProviderCredentials(
  provider: ServiceProvider,
  draft: CredentialDraft,
): ProviderCredentialsInput | null {
  return buildProviderInput(provider, draft, []);
}

/** Empty saved secret slots are resolved natively for a temporary check. */
export function buildProviderProbeCredentials(
  provider: ServiceProvider,
  draft: CredentialDraft,
  savedFields: readonly string[] = [],
): ProviderCredentialsInput | null {
  return buildProviderInput(provider, draft, savedFields);
}

function buildProviderInput(provider: ServiceProvider, draft: CredentialDraft, savedFields: readonly string[]): ProviderCredentialsInput | null {
  if (provider === "appleSpeech") return null;
  const values = Object.fromEntries(
    Object.entries(draft).map(([key, value]) => [key, value.trim()]),
  ) as CredentialDraft;
  if (
    credentialFieldsForProvider(provider).some((field) => field !== "token" && !values[field] && !savedFields.includes(field))
  ) {
    return null;
  }

  switch (provider) {
    case "customDashScopeASR":
    case "customOpenAIASR":
      return { kind: "customSpeech", endpoint: values.endpoint, model: values.model, apiKey: values.apiKey };
    case "deepLX":
      return { kind: "deepLX", asrApiKey: values.asrApiKey, endpoint: values.endpoint, token: values.token };
    case "azureOpenAIRealtime":
      return {
        kind: "azureOpenAI",
        endpoint: values.endpoint,
        deployment: values.deployment,
        transcriptionDeployment: values.transcriptionDeployment,
        apiKey: values.apiKey,
      };
    case "tencentCloud":
      return {
        kind: "tencentCloud",
        appId: values.appId,
        secretId: values.secretId,
        secretKey: values.secretKey,
      };
    case "baiduTranslate":
      return {
        kind: "baiduTranslate",
        appId: values.appId,
        appKey: values.appKey,
      };
    default:
      return { kind: "apiKey", apiKey: values.apiKey };
  }
}

export const CHATMOCK_DEFAULT_ENDPOINT = "http://127.0.0.1:8000/v1";

/** Alibaba retains its profile-scoped key; an empty replacement reuses it natively. */
export function buildAlibabaTranslationCredentials(profile: ServiceProfile, draft: CredentialDraft, translation: TextTranslation, clearToken = false): ProviderCredentialsInput | null {
  const custom = isStandaloneAsrProvider(profile.provider);
  if (!custom && profile.provider !== "alibabaCloud" && profile.provider !== "deepLX") return null;
  if (!custom && !draft.apiKey.trim() && (profile.speechCredentialState ?? profile.credentialState) !== "present") return null;
  const savedTranslation = profile.textTranslation ?? (profile.provider === "deepLX" ? "deepLX" : "followService");
  const keepsSavedDestination = (custom ? profile.textCredentialState : profile.credentialState) === "present" && translation === savedTranslation;
  if (translation === "deepLX" && !draft.endpoint.trim() && !keepsSavedDestination) return null;
  if (translation === "deepL" && !draft.token.trim() && !keepsSavedDestination) return null;
  if (isChatCompletionsTranslation(translation) && !keepsSavedDestination && (!draft.endpoint.trim() || !draft.model.trim())) return null;
  return {
    kind: "alibabaTranslation",
    apiKey: custom ? "" : draft.apiKey.trim(),
    textTranslation: translation,
    endpoint: translation === "deepLX" || isChatCompletionsTranslation(translation) ? draft.endpoint.trim() : "",
    token: translation === "followService" || translation === "apple" || (isChatCompletionsTranslation(translation) && clearToken) ? "" : draft.token.trim(),
    model: isChatCompletionsTranslation(translation) ? draft.model.trim() : "",
    ...(isChatCompletionsTranslation(translation) && clearToken ? { clearToken: true } : {}),
  };
}

export function buildCustomSpeechCredentials(profile: ServiceProfile, draft: Pick<CredentialDraft, "endpoint" | "model" | "apiKey">): ProviderCredentialsInput | null {
  if (!isCustomSpeechProvider(profile.provider)) return null;
  const endpoint = draft.endpoint.trim(), model = draft.model.trim(), apiKey = draft.apiKey.trim();
  const saved = profile.speechCredentialState === "present";
  if ((!saved && (!endpoint || !model || !apiKey)) || (endpoint && (!apiKey || !model))) return null;
  if (saved && !endpoint && !model && !apiKey) return null;
  return { kind: "customSpeech", endpoint, model, apiKey };
}

/** A full credential-free WebSocket URL; plaintext is restricted to loopback. */
export function customSpeechEndpointIsValid(value: string): boolean {
  if (new TextEncoder().encode(value).length > 2048 || Array.from(value).some(char => { const code = char.codePointAt(0)!; return code < 32 || (code >= 127 && code <= 159); })) return false;
  try {
    const url = new URL(value.trim());
    const local = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
    return (url.protocol === "wss:" || (url.protocol === "ws:" && local)) && !!url.hostname &&
      !url.username && !url.password && !value.includes("?") && !value.includes("#");
  } catch { return false; }
}

/** Mirrors native text translation endpoint safety checks before credential I/O. */
export function deepLXEndpointIsValid(value: string): boolean {
  if (new TextEncoder().encode(value).length > 2048 || Array.from(value).some((char) => { const code = char.codePointAt(0)!; return code < 32 || (code >= 127 && code <= 159); })) return false;
  try {
    const url = new URL(value.trim());
    const local = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
    return (url.protocol === "https:" || (url.protocol === "http:" && local)) &&
      !!url.hostname && !url.username && !url.password && !value.includes("?") && !value.includes("#");
  } catch {
    return false;
  }
}

export function openAICompatibleModelIsValid(value: string): boolean {
  return !!value.trim() && new TextEncoder().encode(value).length <= 256 &&
    !Array.from(value).some((char) => { const code = char.codePointAt(0)!; return code < 32 || (code >= 127 && code <= 159); });
}
