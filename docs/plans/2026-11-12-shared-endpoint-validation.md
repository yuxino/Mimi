# Shared text endpoint validation and Android dependency removal

Android's production network clients already run in shared Rust, but the settings
validator still retained OkHttp and Okio solely for `HttpUrl`. Move native endpoint
validation through JNI to the same Rust URL parser and API-path completion used by
OpenAI-compatible and DeepLX requests. Keep configuration models and native UI in
Kotlin; do not add another Kotlin parser or client implementation.

## Boundaries

- Extract the existing portable parser without changing runtime URL acceptance,
  request paths or the global native-platform loopback allowlist.
- Android uses an explicit adapter policy on devices and in host JNI tests. Keep
  its cleartext opt-in, `10.0.2.2` emulator exception, strict URL shape, port-zero
  rejection and existing sanitized error categories. Invalid URL structure still
  precedes transport policy, model and credential errors. Never include input or
  keys in errors or diagnostics.
- Compare Android's local-host allowlist before WHATWG numeric-IP repair. Preserve
  encoded/Unicode hosts and IPv6 normalization without accepting shortened,
  octal, hexadecimal or integer IPv4 aliases. Reuse `idna` and `percent-encoding`,
  already locked and linked through `url`; the direct declarations add no new
  package/version and avoid an independent host decoder. Skip additional forward
  slashes before the authority, as both existing URL parsers do; this does not
  normalize numeric aliases before checking the local-host allowlist.
- Delegate DeepL's fixed endpoint selection to its existing shared implementation.
  Keep the native model/key validation order and optional-key behavior. Bound
  values before JSON serialization into JNI; the shared parser also validates the
  byte limit. Do not save normalized endpoint strings over the user's draft.
- Keep ABI coverage, resources, languages, capture and provider semantics. Leave
  resource/code shrinking and native-library compression for separate measured
  changes; neither is needed to remove this unused HTTP stack.

## Intentional corrections to the obsolete validator

The shared fixtures cover 50 editor-compatibility cases for URL normalization,
local-host policy and malformed input, using the previous compiled Android
validator as the baseline. This includes extra authority slashes with cleartext
opt-in both on and off, plus an HTTPS comparison.
An extended offline comparison also identified differences that belong to the
old independent parser, not to the production request path:

- OkHttp accepted `https://999.999/v1`, although shared Rust has always rejected
  that invalid numeric host. Settings now report `translation_endpoint` before
  saving a configuration that cannot produce a request. For its HTTP form, the
  old validator reported `translation_https_required`; the shared parser now
  reports the invalid endpoint first, regardless of cleartext opt-in.
- The old normalized string mapped `faß.example` to `fass.example` and escaped
  path characters `^` and `|`. Shared Rust uses `xn--fa-hia.example` and preserves
  those path characters. The real request already used the Rust forms. The new
  validation result follows that same implementation.

All production calls to `validateTranslationConfiguration` discard its returned
string: `TextTranslationSettings` returns its original configuration;
`SettingsStore.profileConfigured`, `saveProfile` and `saveConfiguration` use it
only to reject invalid input; `ServiceCatalog.runtimeCredentials` serializes the
original trimmed `endpoint`. `normalizeTranslationEndpoint` itself has no other
production caller. Therefore changing those unused normalized strings does not
redirect working requests or rewrite saved drafts. The `runtimeAligned`
fixtures pin both the shared parser result and the actual JNI endpoint identity;
the Kotlin adapter test also checks that credentials retain the original draft.

IPv4-mapped IPv6 HTTP remains an existing editor/runtime gap: OkHttp and the new
Android policy accept a mapped loopback host, but the raw saved draft still
reaches the runtime, whose unchanged allowlist rejects that IPv6 spelling.
That fixture proves editor compatibility, not a successful production request.
Do not broaden the runtime allowlist as part of this dependency removal.

Keep the old differential result as bounded comparison evidence. Do not add a
second URL implementation merely to preserve a validation success for endpoints
the production runtime cannot use.

## Evidence required

The Android endpoint fixtures are checked against the previous compiled OkHttp
validator, direct shared Rust and the real JNI boundary. Cover opt-in on/off,
emulator/loopback/remote addresses, encoded and Unicode hosts/paths, IPv6, raw
userinfo, repaired URL spellings and error ordering. Compare the five documented
runtime-alignment cases to the old validator separately rather than claiming
exact historical string or error equivalence. Retain the existing shared endpoint,
model, credential and actual-provider JNI suites.

Compare equivalent release APKs and DEX contents with identical build settings.
Run both Android unit/lint variants, packaging guards, canonical checks and native
configuration validation. Package-size changes are artifact measurements, not
claims about live subtitle latency, total memory or translation quality.
