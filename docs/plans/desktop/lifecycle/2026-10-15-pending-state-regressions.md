# Pending editor, recording and recognition state

Baseline: `8a3c8de9`. This audit continues the earlier compatibility review and
adds Android emulator acceptance without provider calls or audio capture.

- Desktop metadata names need an editor-owner lifecycle. Navigating away must
  flush pending saves without losing a failed draft or retry. Keys distinguish
  profile, recognition and translation-route names; confirmed deletion cancels
  queued writes. Credential fields keep their existing separate lifecycle.
- Pending recording finalization may reuse only a verified regular canonical WAV
  matching its PCM journal, including bounded streaming payload comparison.
  A directory, symlink, damaged header or mismatched output must fail and preserve
  the journal for retry. Successful prior source
  output remains reusable when a second source fails.
- An identified custom-ASR completion with empty text withdraws that turn's draft.
  Shared Rust must retire only matching stop-fallback and preview work, including
  late preview callbacks. Confirmed history, newer turns and normal nonempty or
  missing-final stop handling remain intact. Desktop and production Android JNI
  use this same implementation and shared behavioral fixtures.
- Android configuration recreation must preserve the unsaved editor in its own
  RAM-only ViewModel, bound to profile/provider identity. Retain independent
  translation-route fields, pending key-removal intent and model selection;
  restore neither reveal state nor connection-check work/results. Do not put
  credentials in Bundle/SavedStateHandle or disk. Back, successful save/delete
  and finish release the draft; process death reloads the persisted source.

Verification includes old-code failing reproductions, normal-path controls,
independent review, signed macOS UI acceptance, direct Rust and actual JNI tests,
Android native emulator checks, and the canonical repository checks. Synthetic
local HTTP/WebSocket and PCM inputs do not establish live-provider, audio-capture
or physical-device acceptance.
