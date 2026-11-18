import { expect, it } from "vitest";
import { DEFAULT_NETWORK_PROXY, networkProxyConfigKey, validateNetworkProxy } from "./networkProxy";

it("defaults to system and discards unused addresses for system/direct routes", () => {
  expect(DEFAULT_NETWORK_PROXY).toEqual({ mode: "system", url: null });
  for (const mode of ["system", "direct"] as const) {
    expect(validateNetworkProxy(mode, "http://synthetic:secret@example.test")).toEqual({ config: { mode, url: null } });
    expect(networkProxyConfigKey({ mode, url: "ignored" })).toBe(networkProxyConfigKey({ mode, url: null }));
  }
});

it.each([
  [" http://127.0.0.1:7890 ", "http://127.0.0.1:7890/"],
  ["socks5://127.0.0.1", "socks5://127.0.0.1:1080"],
  ["socks5h://[::1]:1081", "socks5h://[::1]:1081"],
])("normalizes a supported custom proxy %s", (address, normalized) => {
  expect(validateNetworkProxy("custom", address)).toEqual({ config: { mode: "custom", url: normalized } });
});

it.each(["", "proxy", "http://127.0.0.1:0", "http://127.0.0.1/path", "http://127.0.0.1?", "http://127.0.0.1#", "http://127.0.0.1\n:80", "http://" + "a".repeat(2_048)])("rejects malformed/path/query/control/oversized endpoints without echoing %s", address => {
  expect(validateNetworkProxy("custom", address)).toEqual({ error: "invalidUrl" });
});

it.each(["http://user:synthetic-secret@127.0.0.1", "socks5://user@127.0.0.1", "http://:synthetic-secret@127.0.0.1", "http://user:@127.0.0.1", "http://@127.0.0.1"])("rejects authenticated proxy addresses without preserving their contents", address => {
  expect(validateNetworkProxy("custom", address)).toEqual({ error: "authenticationUnsupported" });
});

it.each(["https://127.0.0.1", "ftp://127.0.0.1", "socks4://127.0.0.1"])("rejects unsupported schemes %s", address => {
  expect(validateNetworkProxy("custom", address)).toEqual({ error: "unsupportedScheme" });
});
