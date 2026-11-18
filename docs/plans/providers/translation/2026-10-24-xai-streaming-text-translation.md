# Independent xAI recognition and text translation

**Goal:** Add xAI streaming recognition and selectable Grok text translation
through the existing independent recognition/translation design.

**Architecture:** Shared Rust owns every recognizer, typed text destination,
bounded queue, language intersection, proxy, probe and recovery policy. Native
desktop and Android adapters edit the same route contract. No second pipeline
or provider-specific translation scheduler is introduced.

- Keep xAI speech mode as streaming ASR or legacy Grok Voice. New xAI profiles
  select streaming ASR and an independent xAI text destination. Existing Voice
  profiles retain their mode and hide, without deleting, the text selection.
- Treat xAI ASR like other independent recognizers: choose any supported text
  destination, or Original only. Other supported ASR profiles can choose Grok.
  Preserve native feature differences; Android does not acquire desktop-only
  Apple/custom recognition entry points through this change.
- Add typed `xAI` text credentials with an independently stored key and model,
  a fixed endpoint, and default `grok-4.3`. Reuse Chat Completions transport and
  the ordinary text-service form, save, check and error behavior. Users can put
  their existing xAI key in both stages or use different keys. Normal operation
  never borrows authentication from another stage or profile.
- Upgrade old unreleased STT profiles once within their own profile: retain
  their chosen model, copy an existing speech key if present, durably verify the
  text destination, then persist a completion marker. Retry storage failures;
  keep a model editable when its key is missing. Deletion, Original selection,
  route changes and later speech-key replacement must not repopulate text keys.
- Intersect recognition and selected translation language catalogs. Original
  retains the recognition catalog and makes no text request. Keep network
  routes, draft checks, cancellation and final ownership independent.
- Reuse service identity for subtitles: one badge for xAI recognition plus Grok,
  separate badges when recognition and translation providers differ.

**Acceptance:** Confirmed chunks translate before speech-final or Stop; drafts
cannot overwrite finals; cumulative summaries do not duplicate translations;
clear/reconnect fence stale events. Cover free combinations, isolated keys and
proxies, independent checks, migration failure/retry/deletion, route restoration,
common form behavior, and both badge states through Rust, actual JNI, frontend
and native UI checks. Live xAI latency/quality still needs an explicitly
configured development account; synthetic protocols cannot establish it.

References: [xAI STT](https://docs.x.ai/developers/model-capabilities/audio/speech-to-text),
[Chat Completions](https://docs.x.ai/developers/model-capabilities/legacy/chat-completions),
[model migration](https://docs.x.ai/developers/migration/may-15-retirement).
