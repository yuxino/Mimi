import type { NetworkProxyConfig, NetworkProxyMode } from "../types";

export const DEFAULT_NETWORK_PROXY: Readonly<NetworkProxyConfig> = Object.freeze({ mode: "system", url: null });
export type NetworkProxyValidationError = "invalidUrl" | "unsupportedScheme" | "authenticationUnsupported";
export type NetworkProxyValidation = { config: NetworkProxyConfig } | { error: NetworkProxyValidationError };

/** UI validation mirrors the native endpoint boundary. Only the native save
 * persists the route; this never reads OS settings or makes a network call. */
export function validateNetworkProxy(mode: NetworkProxyMode, address: string | null): NetworkProxyValidation {
  if (mode !== "custom") return { config: { mode, url: null } };
  const value = address?.trim() ?? "";
  const hasControl = Array.from(value).some(character => {
    const code = character.codePointAt(0)!;
    return code < 0x20 || (code >= 0x7f && code <= 0x9f);
  });
  if (value === "" || new TextEncoder().encode(value).length > 2_048 || hasControl) return { error: "invalidUrl" };
  let endpoint: URL;
  try { endpoint = new URL(value); } catch { return { error: "invalidUrl" }; }
  const authority = value.slice(value.indexOf("//") + 2).split(/[/?#]/u)[0];
  if (endpoint.username !== "" || endpoint.password !== "" || authority.includes("@")) return { error: "authenticationUnsupported" };
  if (!["http:", "socks5:", "socks5h:"].includes(endpoint.protocol)) return { error: "unsupportedScheme" };
  if (endpoint.hostname === "" || !["", "/"].includes(endpoint.pathname) || value.includes("?") || value.includes("#") || endpoint.port === "0") return { error: "invalidUrl" };
  if (endpoint.port === "" && endpoint.protocol.startsWith("socks5")) endpoint.port = "1080";
  return { config: { mode, url: endpoint.toString() } };
}

export function networkProxyConfigKey(config: NetworkProxyConfig): string {
  return JSON.stringify([config.mode, config.mode === "custom" ? config.url : null]);
}
