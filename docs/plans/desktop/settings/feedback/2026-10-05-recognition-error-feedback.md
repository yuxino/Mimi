# Persistent recognition error feedback

An unsupported source language from a custom Audio3 service was reduced to an
unknown task failure. The overlay could hide its reason behind existing subtitle
content or a collapsed/immersive presentation; other controls primarily offered
retry. This left a configuration failure looking like a transient connection fault.

## Decision

- Whitelist the exact Audio3 `UNSUPPORTED_LANGUAGE` code and map it to a fixed,
  localized explanation. Never render provider error bodies. Connection checks
  expose the typed `unsupportedLanguage` reason; content-free diagnostics retain
  the configuration category and allowlisted code.
- Map Audio3's exact fixed transport/timeout labels to localized speech-connection
  guidance, including checking that a local service is started. These labels survive
  the recognition/translation adapter unchanged; unknown native errors remain hidden.
  Preserve retry without claiming a specific TCP, TLS or proxy cause.
- Treat fixed authentication/unsupported-language and setup request errors as
  requiring user action, including during an existing reconnect loop. Runtime
  generic CLIENT_ERROR remains retryable because it may represent queue overload.
  Explicit LOCAL_ASR_OVERLOADED and LOCAL_ASR_TIMEOUT codes get fixed localized
  recovery advice and retain retry, as do transport, timeout and service errors. Do not infer language support
  from provider names or restrict every custom service to one model's languages.
- A successful source/profile/speech-route change, or an explicit successful save
  of the active service, retires its observed configuration error to idle. Compare
  the original error and lifecycle epoch under the settings guard, retain current
  subtitles, and never start capture automatically. Failed saves, display changes,
  inactive-profile edits and newer session errors do not retire the failure.
- Use a shared normal-size session-error component in settings, tray and overlay
  controls, with an explicit speech-settings action. A configuration error's main
  action opens settings; transient errors retain manual retry. Keep current
  subtitle content visible alongside the failure.
- An errored overlay temporarily uses its existing expanded frame, with a minimum
  280-pixel height, and restores background and click interaction. Native geometry
  applies on the main loop, invalidates old resize transactions, and rejects
  persistence of this temporary frame. Stored collapsed, immersive, locked and
  expanded-size preferences remain unchanged. Recovery restores those preferences.
  Explicit dragging remains available during the override and updates position only,
  retaining the normal saved width and height. Automatic error expansion is never
  promoted to user geometry. Reconcile a pending drag before recovery so a quick
  settings correction cannot snap the window back. Resizing, collapse, entering
  immersive/locked mode and automatic Space following remain suspended while the
  error needs to be readable. Top actions use the same temporary unlocked state as
  the native window; saved reading-mode preferences remain unchanged.

## Error ownership between overlay and controls

The subtitle canvas shows a short localized cause. With the control panel closed,
it offers one primary recovery action: settings for configuration failures, retry
for transient failures. Opening the panel leaves the cause on the canvas and moves
the prominent recovery controls to the panel, which retains the full explanation.
No additional instructional line is added to the canvas. Existing top icons and
error-state dragging remain available. Tray and settings are independent surfaces
and always retain their full error explanation and recovery controls.

Both overlay windows observe the existing native control-mode event. Register the
listener before reading the current mode, and ignore a read if a newer event arrives
while it is pending. Listener/read failures retain the canvas recovery entry rather
than hiding it. Audio3 summaries use the same strict error-code allowlist as the
full messages, with separate Chinese, English and Japanese short causes. Other
adapters already supply safe localized prose: a shared helper uses its first complete
sentence, never arbitrary provider text or character-count truncation.

This presentation follow-up is source-only at the user's request. Regression sources
cover open/closed panel ownership, independent tray feedback, mode-read ordering and
localized safe summaries; no tests, build, native acceptance or CI have run for it.

## Scope and verification

Desktop Audio3 decoding, session recovery and native presentation are affected.
Android currently uses the separate DashScope realtime `session.update` adapter,
not Audio3 `run-task`/`task-failed`; no shared subtitle or text-translation contract
changes. Regression sources cover safe code decoding, loopback setup rejection,
recovery policy, three-language feedback across sibling surfaces and temporary
native geometry. Existing profile-selection regressions cover live/paused/idle
selection, failed persistence and lifecycle supersession; error-retirement
regressions preserve subtitles and reject changed epochs or unrelated failures.
The latest presentation follow-up and integration have not been tested or built.
Native error dragging, mode restoration and real service sessions remain pending
until the user asks for the combined verification pass.

## Integration with live profile selection

The main branch now routes profile selection through `switch_profile` so a live
session reconnects and a paused session keeps its resumable configuration. Keep
that lifecycle path and retire an observed configuration error only after its
new profile selection persists successfully under the same settings guard.
Selecting the same profile, rejecting a missing profile, failed persistence and
a newer lifecycle/error must leave the old error intact. Error recovery keeps
confirmed subtitles and does not start audio. This source-only integration has
not been tested, built, run natively or sent to CI at the user's request.
