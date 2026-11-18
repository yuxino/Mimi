/** Browser-only profile catalog projections. Native state and acknowledgments
 * remain in store.ts; these previews neither persist credentials nor open devices. */
import { SOURCE_LANGUAGE_CODES } from "../types";
import type {
  ProfileOptionsDraft,
  ProviderCredentialsInput,
  ServiceProvider,
  SettingsSnapshot,
  SourceLanguage
} from "../types";
import {
  capabilitiesForProvider,
  capabilitiesForProfile,
  effectiveProviderForProfile,
  isChatCompletionsTranslation,
  isCustomSpeechProvider,
  isIndependentAsrProfile,
  supportsSpeechTranslationMode,
  textTranslationForProfile,
  sourceLanguagesForSettings,
  targetLanguagesForSettings,
} from "../providers/providerCapabilities";
import { openAICompatibleModelIsValid } from "../providers/providerCredentials";

export function createPreviewProfile(
  current: SettingsSnapshot,
  provider: ServiceProvider,
  name: string,
): SettingsSnapshot {
  if(current.profiles.length >= 20) throw new Error("profile-limit");
  const id = `mock-${provider}-${Date.now()}`;
  const snapshot: SettingsSnapshot = {
    ...current,
    activeProfileId: current.profiles.length ? current.activeProfileId : id,
    profiles: [
      ...current.profiles,
      {
        id,
        name,
        provider,
        credentialState: "missing",
        ...(supportsSpeechTranslationMode(provider)
          ? { speechTranslationMode: "simultaneous" as const }
          : {}),
        ...(provider === "xAIRealtime"
          ? {
            xaiSpeechMode: "asrMt" as const,
            textTranslation: "xAI" as const,
            xaiIndependentTranslationMigrated: true,
            speechCredentialState: "missing" as const,
            textCredentialState: "missing" as const
          }
          : {}),
        ...(provider === "alibabaCloud"
          ? { alibabaRealtimeMode: "liveTranslate" as const, alibabaWorkspaceId: "" }
          : {}),
        ...(isCustomSpeechProvider(provider)
          ? { speechCredentialState: "missing" as const, textCredentialState: "missing" as const }
          : {})
      },
    ],
  };
  return snapshot;
}

export function duplicatePreviewProfile(current: SettingsSnapshot, profileId: string, name: string): SettingsSnapshot {
  if(current.profiles.length >= 20) throw new Error("profile-limit");
  const source = current.profiles.find(profile => profile.id === profileId);
  if(!source) throw new Error("profile-not-found");
  const copy = structuredClone(source);
  copy.id = `mock-copy-${crypto.randomUUID()}`;
  copy.name = name;
  const snapshot = { ...current, profiles: [...current.profiles, copy] };
  return snapshot;
}

export function updatePreviewProfile(
  current: SettingsSnapshot,
  profileId: string,
  name: string | undefined,
  options?: ProfileOptionsDraft
): SettingsSnapshot {
  const hasDeclaration = options !== undefined && Object.hasOwn(options, "customSpeechSourceLanguages");
  if(options?.qwenMtModel !== undefined) {
    const profile = current.profiles.find(profile => profile.id === profileId);
    if(profile?.provider !== "alibabaCloud"
      || !["lite", "flash", "plus"].includes(options.qwenMtModel)) {
      throw new Error("provider-mismatch");
    }
  }
  if(options?.alibabaRealtimeMode !== undefined || options?.alibabaWorkspaceId !== undefined) {
    const profile = current.profiles.find(profile => profile.id === profileId);
    if(profile?.provider !== "alibabaCloud") throw new Error("provider-mismatch");
    const mode = options.alibabaRealtimeMode ?? profile.alibabaRealtimeMode ?? "disabled";
    const workspace = (options.alibabaWorkspaceId ?? profile.alibabaWorkspaceId ?? "").trim();
    if(!["disabled", "liveTranslate"].includes(mode) || workspace.length > 63
      || (workspace !== "" && !/^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$/.test(workspace))
      || (mode !== "disabled" && workspace === "")) {
      throw new Error("alibaba-realtime-invalid");
    }
  }
  if(options?.xaiSpeechMode !== undefined) {
    const profile = current.profiles.find(profile => profile.id === profileId);
    if(profile?.provider !== "xAIRealtime") throw new Error("provider-mismatch");
    if(!["voice", "asrMt"].includes(options.xaiSpeechMode)) throw new Error("provider-mismatch");
  }
  if(options?.speechTranslationMode !== undefined) {
    const profile = current.profiles.find(profile => profile.id === profileId);
    if(!profile || !supportsSpeechTranslationMode(profile.provider)
      || !["simultaneous", "asrMt"].includes(options.speechTranslationMode)) {
      throw new Error("provider-mismatch");
    }
  }
  const declaration = options?.customSpeechSourceLanguages ?? null;
  if(hasDeclaration) {
    const profile = current.profiles.find(profile => profile.id === profileId);
    if(!profile || !isCustomSpeechProvider(profile.provider)) throw new Error("provider-mismatch");
    if(declaration !== null
      && (declaration.length > SOURCE_LANGUAGE_CODES.length || declaration.includes("auto"))) {
      throw new Error("custom-speech-languages-invalid");
    }
  }
  if(options && Object.hasOwn(options, "languagePreset")) {
    const profile = current.profiles.find(profile => profile.id === profileId);
    if(!profile) throw new Error("profile-read-only");
    const preset = options.languagePreset;
    if(preset && profile.provider !== "appleSpeech"
      && textTranslationForProfile(profile) !== "apple") {
      const proposed = { ...current, ...preset, activeProfileId: profileId, languageCapabilities: undefined };
      if(!sourceLanguagesForSettings(proposed).includes(preset.sourceLanguage)
        || !targetLanguagesForSettings(proposed).includes(preset.targetLanguage)) {
        throw new Error("profile_language_preset_unsupported");
      }
    }
  }
  const snapshot: SettingsSnapshot = {
    ...current,
    profiles: current.profiles.map(
      (profile) =>
        profile.id === profileId
          ? (() => {
            const { textTranslationName, speechRecognitionName, ...proxies } = options ?? {};
            const textTranslationNames = { ...profile.textTranslationNames };
            if(textTranslationName) {
              const value = textTranslationName.name.trim();
              if(value) textTranslationNames[textTranslationName.route] = value;
              else delete textTranslationNames[textTranslationName.route];
            }
            const next = {
              ...profile,
              ...(name === undefined ? {} : { name: name.trim() }),
              ...proxies,
              textTranslationNames,
              ...(options?.alibabaWorkspaceId === undefined
                ? {}
                : { alibabaWorkspaceId: options.alibabaWorkspaceId.trim() }),
              ...(hasDeclaration
                ? {
                  customSpeechSourceLanguages: declaration === null
                    ? null
                    : SOURCE_LANGUAGE_CODES.filter(code => declaration.includes(code))
                }
                : {}),
              ...(speechRecognitionName === undefined
                ? {}
                : { speechRecognitionName: speechRecognitionName.trim() || undefined })
            };
            if(options?.speechTranslationMode !== undefined) {
              next.speechCredentialState = profile.speechCredentialState ?? profile.credentialState;
              next.textCredentialState = profile.textCredentialState ?? "missing";
              next.credentialState = !isIndependentAsrProfile(next)
                || textTranslationForProfile(next) === "followService"
                || next.textCredentialState === "present"
                ? next.speechCredentialState
                : next.speechCredentialState === "unavailable" ? "unavailable" : "missing";
            }
            return next;
          })()
          : profile,
    ),
  };
  if((hasDeclaration || options?.alibabaRealtimeMode !== undefined
    || options?.xaiSpeechMode !== undefined
    || options?.speechTranslationMode !== undefined)
    && current.activeProfileId === profileId) {
    if(options?.alibabaRealtimeMode !== undefined || options?.xaiSpeechMode !== undefined
      || options?.speechTranslationMode !== undefined) {
      snapshot.languageCapabilities = undefined;
    }
    const sources = sourceLanguagesForSettings(snapshot);
    if(!sources.includes(snapshot.sourceLanguage)) snapshot.sourceLanguage = sources[0]!;
    if(options?.alibabaRealtimeMode !== undefined || options?.xaiSpeechMode !== undefined
      || options?.speechTranslationMode !== undefined) {
      const targets = targetLanguagesForSettings(snapshot);
      if(!targets.includes(snapshot.targetLanguage)) snapshot.targetLanguage = targets[0]!;
    }
  }
  return snapshot;
}

export function selectPreviewProfile(
  current: SettingsSnapshot,
  profileId: string,
  sourceLanguage?: SourceLanguage,
): SettingsSnapshot {
  const selected = current.profiles.find((profile) => profile.id === profileId);
  if(!selected) throw new Error("profile-not-found");
  const preset = sourceLanguage ? null : selected.languagePreset;
  if(preset) {
    const proposed = { ...current, ...preset, activeProfileId: profileId, languageCapabilities: undefined };
    if(!sourceLanguagesForSettings(proposed).includes(preset.sourceLanguage)
      || !targetLanguagesForSettings(proposed).includes(preset.targetLanguage)) {
      throw new Error("profile_language_preset_unsupported");
    }
  }
  const snapshot = settingsAfterPreviewProfileSelection(current, effectiveProviderForProfile(selected));
  if(isIndependentAsrProfile(selected)) {
    const capabilities = capabilitiesForProfile(selected, current.targetLanguage);
    snapshot.sourceLanguage = capabilities.sourceLanguages.includes(current.sourceLanguage)
      ? current.sourceLanguage
      : capabilities.sourceLanguages[0] ?? current.sourceLanguage;
    snapshot.targetLanguage = capabilities.targetLanguages.includes(current.targetLanguage)
      ? current.targetLanguage
      : capabilities.targetLanguages[0]!;
  }
  if(sourceLanguage) snapshot.sourceLanguage = sourceLanguage;
  if(preset) Object.assign(snapshot, preset);
  snapshot.activeProfileId = profileId;
  return snapshot;
}

export function deletePreviewProfile(current: SettingsSnapshot, profileId: string): SettingsSnapshot {
  if(current.profiles.length <= 1) throw new Error("last-profile");
  const profiles = current.profiles.filter((profile) => profile.id !== profileId);
  const activeProfileId =
    current.activeProfileId === profileId
      ? (profiles[0]?.id ?? "")
      : current.activeProfileId;
  const snapshot: SettingsSnapshot = { ...current, profiles, activeProfileId };
  return snapshot;
}

export function savePreviewCredentials(
  current: SettingsSnapshot,
  profileId: string,
  credentials: ProviderCredentialsInput,
): SettingsSnapshot {
  if(credentials.kind === "alibabaTranslation") {
    const profile = current.profiles.find((profile) => profile.id === profileId);
    if(!profile
      || (!isIndependentAsrProfile(profile)
        && !["alibabaCloud", "deepLX"].includes(profile.provider))) {
      throw new Error("provider-mismatch");
    }
    if(isIndependentAsrProfile(profile)
      ? credentials.apiKey.trim()
      : !credentials.apiKey.trim()
      && (profile.speechCredentialState ?? profile.credentialState) !== "present") {
      throw new Error("credential-empty");
    }
    if(credentials.textTranslation === "apple"
      && (credentials.endpoint.trim() || credentials.token.trim() || credentials.model.trim())) {
      throw new Error("provider-mismatch");
    }
    if(credentials.textTranslation === "deepLX" && !credentials.endpoint.trim()
      && textTranslationForProfile(profile) !== "deepLX") {
      throw new Error("credential-empty");
    }
    if(credentials.textTranslation === "deepL" && !credentials.token.trim()
      && textTranslationForProfile(profile) !== "deepL") {
      throw new Error("credential-empty");
    }
    if(credentials.textTranslation === "xAI" && credentials.model
      && !openAICompatibleModelIsValid(credentials.model)) {
      throw new Error("invalid-openai-compatible-model");
    }
    if(credentials.textTranslation === "xAI" && credentials.endpoint) throw new Error("provider-mismatch");
    if(credentials.textTranslation === "xAI"
      && (textTranslationForProfile(profile) !== "xAI" || profile.textCredentialState !== "present")
      && (!credentials.token.trim() || !credentials.model.trim())) {
      throw new Error("credential-empty");
    }
    if(isChatCompletionsTranslation(credentials.textTranslation)
      && textTranslationForProfile(profile) !== credentials.textTranslation
      && (!credentials.endpoint.trim() || !credentials.model.trim())) {
      throw new Error("credential-empty");
    }
  } else if(credentials.kind === "customSpeech") {
    const profile = current.profiles.find(profile => profile.id === profileId);
    if(!profile || !isCustomSpeechProvider(profile.provider)) throw new Error("provider-mismatch");
    if((profile.speechCredentialState !== "present"
      && (!credentials.endpoint.trim() || !credentials.model.trim() || !credentials.apiKey.trim()))
      || (credentials.endpoint.trim() && !credentials.apiKey.trim())) {
      throw new Error("credential-empty");
    }
  } else if(Object.entries(credentials).some(
    ([field, value]) => field !== "kind" && field !== "token"
      && !(field === "deployment" && credentials.kind === "azureOpenAI"
        && current.profiles.some(profile => profile.id === profileId && profile.speechTranslationMode === "asrMt"))
      && !value.trim()
  )) {
    throw new Error("credential-empty");
  }
  const snapshot: SettingsSnapshot = {
    ...current,
    profiles: current.profiles.map(
      (profile) =>
        profile.id === profileId
          ? {
            ...profile,
            credentialState: "present",
            ...(credentials.kind === "alibabaTranslation"
              ? {
                textTranslation: credentials.textTranslation,
                ...(isIndependentAsrProfile(profile)
                  ? {
                    textCredentialState: "present" as const,
                    credentialState: profile.speechCredentialState ?? "missing"
                  }
                  : {})
              }
              : {}),
            ...((credentials.kind === "customSpeech"
              || (credentials.kind === "apiKey" || credentials.kind === "azureOpenAI")
              && (isIndependentAsrProfile(profile) || supportsSpeechTranslationMode(profile.provider)))
              ? {
                speechCredentialState: "present" as const,
                credentialState: !isIndependentAsrProfile(profile)
                  || textTranslationForProfile(profile) === "followService"
                  || profile.textCredentialState === "present"
                  ? "present" as const
                  : "missing" as const
              }
              : {})
          }
          : profile,
    ),
  };
  if(current.activeProfileId === profileId && credentials.kind === "alibabaTranslation") {
    snapshot.languageCapabilities = undefined;
    const sources = sourceLanguagesForSettings(snapshot);
    if(!sources.includes(snapshot.sourceLanguage)) {
      snapshot.sourceLanguage = sources[0] ?? snapshot.sourceLanguage;
    }
    const targets = targetLanguagesForSettings(snapshot);
    if(!targets.includes(snapshot.targetLanguage)) {
      snapshot.targetLanguage = targets[0] ?? snapshot.targetLanguage;
    }
  }
  return snapshot;
}

export function deletePreviewCredentials(current: SettingsSnapshot, profileId: string): SettingsSnapshot {
  const snapshot: SettingsSnapshot = {
    ...current,
    profiles: current.profiles.map(
      (profile) =>
        profile.id === profileId
          ? {
            ...profile,
            credentialState: "missing",
            ...(isCustomSpeechProvider(profile.provider)
              || supportsSpeechTranslationMode(profile.provider)
              || isIndependentAsrProfile(profile) && profile.provider !== "appleSpeech"
              ? { speechCredentialState: "missing" as const, textCredentialState: "missing" as const }
              : profile.provider === "appleSpeech" ? { textCredentialState: "missing" as const } : {})
          }
          : profile,
    ),
  };
  return snapshot;
}
function settingsAfterPreviewProfileSelection(
  current: SettingsSnapshot,
  provider: ServiceProvider,
): SettingsSnapshot {
  if(provider === "alibabaCloud") return { ...current };
  const capabilities = capabilitiesForProvider(provider);
  const sourceLanguage =
    capabilities.sourceLanguages.find(
      (source) =>
        !(
          (source === "zh" && current.targetLanguage === "zh")
          ||
          (source === "en" && current.targetLanguage === "en")
          ||
          (source === "ja" && current.targetLanguage === "ja")
        ),
    )
    ?? capabilities.sourceLanguages[0];
  return {
    ...current,
    sourceLanguage: sourceLanguage ?? current.sourceLanguage,
    targetLanguage:
      current.targetLanguage === "original" ? "zh" : current.targetLanguage,
    translationMode: "turbo",
  };
}
