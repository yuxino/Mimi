import { providerDisplayName } from "./i18n";
import { isCustomSpeechProvider } from "./providerCapabilities";
import type { ServiceProfile } from "./types";

/** A display-only alias never changes the selected recognition protocol. */
export function speechRecognitionDisplayName(profile: ServiceProfile): string {
  if (profile.provider === "localSpeech") {
    const names = { qwenSmall: "Qwen3-ASR 0.6B", qwenStandard: "Qwen3-ASR 1.7B" };
    const id = profile.localSpeechModel ?? "qwenSmall";
    return names[id];
  }
  if (isCustomSpeechProvider(profile.provider)) {
    const name = profile.speechRecognitionName?.trim();
    if (name) return name;
  }
  return providerDisplayName(profile.provider === "deepLX" ? "alibabaCloud" : profile.provider);
}
