# Alibaba translation pipeline

## Scope

This record describes the Alibaba Cloud backend that ships with mimi. It is a
provider-specific implementation behind the typed provider facade; OpenAI
Realtime has its own protocol and audio-format contract.

All Alibaba clients use the shared DashScope endpoints and one provider/profile
scoped API key. No Workspace ID participates in configuration or IPC.

## Modes

- **Low latency** uses the DashScope LiveTranslate realtime WebSocket. Automatic
  source detection omits the language hint and routes here unless the user has
  explicitly selected Turbo.
- **High quality** uses Audio 3.0 streaming ASR and translates confirmed source
  chunks with `qwen-mt-plus`. It requires an explicit source language.
- **Turbo** uses the same Audio 3.0 recognizer with `qwen-mt-flash`, a 500 ms
  stable-draft delay, a 2 s maximum wait, and a 12-character long-tail
  threshold. Turbo remains Turbo with automatic source detection.
- **Original subtitles** keep the strongest recognition path available for the
  selected source/mode and do not call machine translation.

Provider capabilities are the authority for source languages, target languages,
translation modes, and the 16 kHz mono PCM input format. Unsupported persisted
preferences are normalized before a session starts and are never silently
accepted by the runtime.

## Durable subtitle rules

Audio 3.0 drafts are replaceable previews. Stable punctuation and the bounded
maximum-wait path may request a translated preview so long uninterrupted speech
continues to flow, but that work is latest-only: it emits `SourceDraft` and
`TranslationDraft`, never advances durable history or translation memory.

Server finals remain authoritative:

- overlap against already confirmed server output is removed without treating
  a later repeated utterance as a global duplicate;
- arrival invalidates any older stable/maximum-wait preview; and
- source and translation are committed together as one atomic
  `SubtitleFinalPair`, so history never observes a half-pair.

The low-latency live-translate pipeline does not pair recognition and
translation finals by arrival order: `conversation.item.created.previous_item_id`
links a translation response item to the source item it answers, and both
streams carry that item id. Drafts stay replaceable previews and are forwarded
without waiting; durable history is committed only once the matched source and
translation finals are available, an empty translation final never consumes
another source utterance, and identity state is generation-local and bounded.
When a new source starts, an older source without its translation stays pending
for a late response. At most 64 source and 64 response identities are retained;
older unmatched items are discarded without falling back to arrival-order
pairing. This bounds long sessions even when the provider omits recognition
events. Preview snapshots expose that identity as a normalized `utteranceId` on
both lines — always the source item — and bilingual live rows combine only when
both lines carry the same identity or neither carries one.
The captured session in `docs/demos/english-film/response.json` completes the
translation first for seven of eight utterances (source first once, +36 ms to
+127 ms apart), which is why arrival order cannot identify an utterance.

The serial final-translation queue contains only authoritative server finals
and an explicit session-finish fallback. It has a hard capacity and maximum
request age. Crossing either bound emits one content-free recoverable overload
error; the session generation is rebuilt instead of discarding confirmed
provider output or growing memory without limit. The final lane has strict
presentation priority: while a final is active or queued, new ASR drafts keep
accumulating in the committer but cannot start a competing preview. Once the
final lane is empty, timers resume from the latest pending draft. Preview work
therefore can neither delay a server final nor interleave translation drafts
from two utterances.

## Translation quality

`QwenMTDomainHint::spoken_dialogue` and its filler glossary are wire assets.
They preserve particles, vocalizations, tone, deliberate repetition, and
explicit dialogue, and they require translation-only output. Do not casually
rewrite these strings.

High-quality server finals use recent source/translation pairs as bounded
translation memory. Preview translations never enter memory. For automatic
recognition, a detected source language is pinned when available. Memory is
context only: remembered lines must not be repeated in the new output.

## Lifecycle and recovery

Session state owns an immutable resolved provider configuration. Pause stops
capture and transport work without clearing durable subtitle history; resume
creates a new guarded generation. Manual stop, provider failure, health checks,
and automatic recovery all use lifecycle generations so stale async work cannot
publish into a newer session.

Health checks use bounded ping/pong timeouts rather than audio-volume inference.
Stop drains already queued audio for a finite interval, closes the provider,
and only accepts provider-confirmed atomic tail pairs while stopping.

## Verification

The Rust suite covers protocol payloads, mode dispatch, sentence commits,
deduplication/replacement, queue bounds, retry ownership, stale generations,
pause/resume, recovery, and bounded shutdown. Run `./scripts/check.sh` after any
pipeline change. Real-provider checks use credentials already stored in the OS
credential store and must not log audio or subtitle content.
