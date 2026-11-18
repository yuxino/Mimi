# DashScope shared endpoints

## Decision

Alibaba Cloud sessions use the shared `dashscope.aliyuncs.com` endpoints and
authenticate with `Authorization: Bearer <api-key>`. Mimi does not request,
store, validate, serialize, or display a Workspace ID.

| Client | Endpoint |
|---|---|
| Audio 3.0 ASR | `wss://dashscope.aliyuncs.com/api-ws/v1/inference` |
| LiveTranslate | `wss://dashscope.aliyuncs.com/api-ws/v1/realtime` |
| Qwen-MT | `https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions` |

Provider-specific dedicated workspace domains are not part of the current
product contract. Supporting one later would require an explicit typed provider
setting and migration; it must not reintroduce a mandatory field for users of
the shared endpoints.

## Boundaries

- Endpoint constructors take no Workspace ID.
- Runtime configuration contains provider, API key, source language, target
  language, and translation mode only.
- Service profile metadata contains no credential and no Workspace ID.
- Frontend snapshots and commands expose credential state or a write-only
  replacement, never the saved key.
- Legacy preference data may be read during migration, but obsolete Workspace
  ID fields are not retained in the current catalog or IPC contract.

Only the hosts changed during the original migration. Audio task payloads,
LiveTranslate session messages, and Qwen-MT `translation_options` remain
provider protocol assets and are covered by protocol tests.

## Verification

Tests assert each fixed endpoint, API-key-only configuration validation, absence
of Workspace ID from current payloads, and secret redaction. Run
`./scripts/check.sh` after changing any endpoint or configuration boundary.
