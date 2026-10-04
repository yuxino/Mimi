import { I18N, providerDisplayName } from "./i18n";
import { textTranslationForProfile } from "./providerCapabilities";
import type { ServiceProfile, TextTranslation } from "./types";

export function defaultTextTranslationName(route: Exclude<TextTranslation, "followService">): string {
  return route === "deepL" ? "DeepL" : route === "deepLX" ? "DeepLX"
    : route === "chatMock" ? "ChatMock" : I18N.settings.textTranslationOpenAICompatible;
}

export function textTranslationDisplayName(profile: ServiceProfile, route = textTranslationForProfile(profile)): string {
  if (route === "followService") return providerDisplayName(profile.provider === "deepLX" ? "alibabaCloud" : profile.provider);
  return profile.textTranslationNames?.[route]?.trim() || defaultTextTranslationName(route);
}
