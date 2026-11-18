# Speech product integration

Integrate recognition error recovery (#160), language configuration (#162),
realtime draft progression (#164), and native Apple Speech (#153) onto the
current product branch. Preserve main's independent recognition names (#161)
and live saved-service switching (#163). This is not a UI redesign or release.

## Boundaries

- Keep the existing system-audio default and explicit microphone choice, per-input
  recognition/subtitle lanes, bounded confirmed history, and recording consent.
- Retain source/target and saved-profile changes, pause/resume/stop cancellation,
  and stale-generation guards. A durable choice can survive reconnect failure;
  its initiating control must receive the failure and must not claim success.
- A failed resume restores the existing paused state under the same lifecycle
  lock used to validate ownership, then returns the error to its initiating
  control. A newer stop wins; failure cannot revive a stopped session. Settings,
  floating controls and tray show only allowlisted causes and ignore failures
  superseded by a newer resume or stop.
- Keep streaming drafts replaceable, confirmed pairs durable, HTTP previews
  paired, and the intermediate-subtitle toggle effective on all routes.
- Keep persistent safe error causes, one canvas recovery action when controls
  are closed, panel-owned detail when open, and independent settings/tray feedback.
  Error presentation may temporarily unlock/expand the window without persisting
  those dimensions or changing saved collapsed/locked/immersive preferences.
- Apple uses the native Speech APIs only on eligible devices. Explicit resource
  preparation and actual recognition-language selection remain separate actions.
  Native languages intersect the selected translator's implemented source codes.
  Runtime capability checks and the Xcode 26+ Apple Silicon build requirement
  remain part of the product; unsupported platforms retain unavailable adapters.

## Exclusions and evidence

External model runners, bridges, model presets, benchmarks, measurement scripts,
model-specific CI and experimental reports from #55 and #154–159 are excluded.
The pre-existing integration ledger stays unchanged; new mixed local-model run
reports are omitted. Apple queue regression source, build support and API/privacy
contracts remain because they support the actual shipped-code path.

The initial pass was source and diff review only. The user subsequently authorized
focused regressions and native testing. Earlier branch measurements remain
separate from the following evidence for this integration; no release is implied.

- Automated regressions: 423 focused frontend tests, all 1,660 frontend tests,
  and 120 focused Rust tests passed. The initial canonical run found an older
  language-selection test that conflicted with the expanded catalog. After its
  parameterization was corrected, the complete check passed: 1,153 Rust tests
  (2 ignored), 75 shared-core tests, 1,660 frontend tests, 6 updater tests,
  8 debugger checks, fmt, clippy, lint, typecheck, production build and diff checks.
- Old installed build `f5859a66334dc954d9ebbfc298f349a4794b0419` (`clean`):
  content-free trace snapshot 505 reached both settings and overlay stores, but
  344 source / 253 translation characters yielded zero selected source characters
  while the overlay stayed on two confirmed blocks. Its scroll position was
  already at the bottom. This locates the observed mismatch in display selection.
- New signed canonical development build
  `311ad4b896d2b5b869c675ee0c0177e47c3d1e35` (`clean`, verified in the UI route):
  Google Gemini, automatic input to Chinese, bilingual, intermediate subtitles on,
  system audio only and microphone off. Replaying the same YouTube segment from
  the beginning showed a new live draft after a final, then continuing drafts
  after pause/resume from both the overlay and the control panel. Snapshot 206
  had 193 source / 136 translation characters, one history entry, 193 selected
  source characters and 337 visible characters. Both windows applied the same
  snapshot and overlay commits continued; scroll geometry remained at the tail.
  Native stop/start reached idle then listening and produced new confirmations.
  A normal quit ended the old process; reopening the same canonical app and
  starting the same replay produced a new live draft with 399 source / 244
  translation characters. The first resume took about 0.8 seconds to restore
  the connection; the UI showed roughly 92–98 ms RTT. Neither measurement is
  audio-to-visible-subtitle latency.
  The [Gemini design](../gemini/2026-10-05-gemini-subtitle-progress.md) records the numeric
  observations without subtitle content.
- The tested binary remains `311ad4b`. Subsequent `a71e7d44` Rust formatting and
  test parameterization, documentation changes and the `ab477597` README guidance
  merged from main do not change Gemini behavior; they are not a newly tested
  binary. These runs do not establish every language, continuous-speech duration
  or platform, nor do they validate Apple Speech recognition or claim
  accuracy/latency gains.
