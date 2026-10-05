# Tencent realtime speech translation repair

## Confirmed contract

Reviewed the official [realtime speech translation WebSocket API](https://cloud.tencent.com/document/product/1093/127565)
on 2026-10-05. This ASR service still requires the account AppID and a
SecretID/SecretKey pair. It does not accept the single API Key used by the
separate Hunyuan text API. Keep the three required fields; Mimi generates the
endpoint, timestamp, nonce, voice ID and signature internally.

The account AppID is available in [account information](https://console.cloud.tencent.com/developer).
The key pair comes from [API key management](https://console.cloud.tencent.com/cam/capi).
The service must be enabled in the [ASR console](https://console.cloud.tencent.com/asr).
Expose these destinations beside Tencent configuration, with secondary
explanations in the existing help UI and matching Chinese, English and Japanese
copy. Do not introduce another credential type or migrate saved credentials.

## Evidenced failures and repair

- Desktop setup collapses Tencent JSON rejection codes into a generic failure;
  Android also discards the distinction. Preserve only sanitized numeric codes
  and classify 6001 (parameters), 6002 (authentication), 6003 (activation),
  6004/6005 (quota/billing) and 6006 (concurrency) into actionable feedback.
  Never display or log the provider's raw message, signed URL or credentials.
- Android sends a partial final audio frame through a protocol adapter that
  requires a full 6,400-byte frame. Pad the bounded tail before the end message,
  matching desktop behavior; empty tails must not add a frame.
- Keep Tencent source and translated final results paired when passing them to
  the existing shared subtitle core. Apply bounded response validation in the
  Android adapter and protect the common behavior with shared provider fixtures.

The supported language selection is unchanged: Tencent's newer language list
has source-dependent target combinations, so a flat expansion would permit
invalid routes. New language support is outside this repair.

## Verification

Use synthetic credentials and documented response shapes in shared fixtures,
Rust WebSocket mocks and Android unit tests. Cover setup rejection, safe unknown
errors, final source/result pairing and audio end boundaries. Run the canonical
desktop check and Android host JNI suite. Inspect the Tencent setup UI in the
signed `/Applications/mimi-dev.app` UI-only mode. These checks do not establish
live-account activation, billing eligibility or real Tencent subtitle latency.
