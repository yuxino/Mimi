# OpenAI-compatible third-party text translation

Issue: [#70](https://github.com/yuxino/mimi/issues/70).

OpenAI-compatible Chat Completions is an independent text-translation route
for speech profiles that support a separate text stage. It translates recognized
sentences; this route alone does not configure third-party ASR or Realtime.
See the [stage settings contract](2026-10-03-service-stage-settings.md).

The supported contract uses optional Bearer authentication and non-streaming
`POST /chat/completions`, with `model` and system/user `messages`, returning
a nonempty string in `choices[0].message.content`. The settings form explains
this contract, accepts a base URL or complete Chat Completions URL, and asks
for the model and a separate translation API key. Examples are placeholders,
never saved defaults. Model language quality is the provider's responsibility.

The service address retains version paths and reverse-proxy prefixes. HTTPS
is required except localhost/loopback HTTP. URL credentials, queries and
fragments are rejected, and redirects are not followed. Requests have an
eight-second deadline, a one-MiB response limit and a 64-KiB translated-text
limit. No dependencies or separate services are introduced.

The adapter reuses the Audio 3.0 pipeline's bounded drafts, cancellation,
session generation guards and ordered final translations. Confirmed history
and optional system-audio files retain their existing default-off behavior.
Diagnostics contain fixed errors, status codes, timings and counts only.

`openAICompatible` identifies this independent text-translation route. ASR
credentials and text destinations use separate profile-scoped slots in the
[private local credential file](2026-10-04-local-credential-storage.md). Switching
routes must preserve the saved DeepL, DeepLX and generic destination values
separately. Empty editor fields may reuse saved values;
credentials are never echoed in settings snapshots. Development uses ordinary
editable profiles in its own private file.

Independent connection checks translate a fixed public test phrase without
requiring or reading the ASR key. Complete editor drafts follow the
[draft-check contract](2026-10-05-draft-connection-checks.md). Synthetic local HTTP
tests cover wire shape, authentication, invalid and empty responses, size limits,
cancellation and proxy routing.
Core/store/UI tests cover persistence, route switching and field validation.
Run the repository check and inspect the signed canonical development app.

Live validation can use Alibaba's OpenAI-compatible service and its matching
regional key. This establishes that the generic path interoperates with one
real provider; it does not prove every private gateway or model. Record the
actual validation and any remaining boundary before delivery.

## Acceptance on 2026-10-03

- The full `./scripts/check.sh` passed: strict Rust formatting/clippy, 825
  Rust tests and 764 frontend tests, updater checks and the production build.
- Real Alibaba Audio 3.0 setup and configured Chat Completions checks passed
  for Japanese and Simplified Chinese targets using `qwen-turbo`. Separate
  translations of a fixed public greeting matched the requested languages;
  their measured request times were 329 ms and 435 ms. Combined setup/probe
  times were 715 ms and 817 ms. These are request checks, not measured
  playback-to-caption latency or a new system-audio capture acceptance.
- The signed canonical macOS development app was inspected in UI-only mode:
  the new selector, protocol explanation and three fields were visible;
  invalid addresses focused the field and preserved drafts, corrections
  removed the error, and successful synthetic saves cleared the inputs.
  UI-only checks remained marked untested and did not access provider
  networks or secure credentials. Floating expanded/collapsed controls and
  active-session configuration locking were also inspected.
- Automatic recognition may report a source language outside the manual
  route picker. The generic request accepts every known detected ASR code;
  a regression covers `fr-FR` and the complete detected-source catalog.
- Live checks used a temporary, ignored development-only harness through the
  existing validated, read-only credential store. The harness was removed;
  private keys, content, local credential paths and response bodies were not
  added to the repository. Production credential storage is unchanged.

Private gateways, other models and live Windows/Linux sessions were not
verified. This change does not publish a new application release or establish
compatibility for the issue author's unspecified private API.

Protocol references:

- [OpenAI Chat Completions](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)
- [Alibaba OpenAI compatibility](https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope)
