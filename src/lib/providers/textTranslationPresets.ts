import { OPENAI_COMPATIBLE_PRESETS } from "../generated/providerCopy";

/** Recognize an editable preset without persisting a second provider identity. */
export function compatiblePresetForEndpoint(endpoint: string) {
  try {
    const url = new URL(endpoint.trim());
    if (url.username || url.password || url.search || url.hash) return undefined;
    url.pathname = url.pathname.replace(/\/+$/, "").replace(/\/chat\/completions$/, "");
    return OPENAI_COMPATIBLE_PRESETS.find(preset => new URL(preset.endpoint).href.replace(/\/+$/, "") === url.href.replace(/\/+$/, ""));
  } catch {
    return undefined;
  }
}
