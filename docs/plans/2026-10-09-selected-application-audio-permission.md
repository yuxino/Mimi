# Selected-application audio-only permission

Follow-up to #230 and v1.5.19. macOS 14.2+ Core Audio process taps support
specified processes as well as the system mix; Screen Recording is not an
Apple requirement for this capability.

- Reuse existing Screen & System Audio Recording grants via ScreenCaptureKit.
- Without a screen grant, both All Applications and selected applications use
  private, unmuted, mono Core Audio taps on 14.2+. Older systems keep SCK.
- Selected capture uses an inclusion list, never a global fallback. Resolve
  exact bundle identity, original app PIDs and nested bundle helper executable
  paths. External XPC helpers (notably WebKit GPU audio) use an exact live
  responsibility PID belonging to the selected original app instance. Refresh
  the bounded process list every 100 ms to follow new helpers.
  App termination reports ApplicationUnavailable and releases native resources.
- Keep identities and paths transient and out of diagnostics. Own-process audio
  remains excluded. Cancellation and teardown ownership stay unchanged.
- Localize the exact permission-denied label in all seven UI languages. The
  existing error surfaces offer a fixed OS privacy-page action and manual Retry;
  they never mutate a permission or redirect users to provider credentials.
- Update the re-openable macOS guide without a special screen-recording warning
  for selected apps. Its explicit settings button opens the same fixed privacy
  page, with a localized manual fallback if the OS opener fails. It performs
  no capture, credential or grant action.

Apple references:
- https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps
- https://developer.apple.com/documentation/coreaudio/catapdescription/initmonomixdownofprocesses:

## Compatibility trade-off

WebKit audio processes identify themselves as WebKit and live outside the host
bundle. Their parent is launchd, so neither the public Core Audio bundle ID,
executable path nor parent PID identifies the host. The optional libSystem SPI
`responsibility_get_pid_responsible_for_pid(pid_t)` supplies that ownership.
Its ABI is also used in [Chromium's implementation](https://github.com/chromium/chromium/blob/main/base/process/process_info_mac.mm).
Resolve it dynamically: unavailable symbols or invalid/unknown responsibility
exclude the helper rather than falling back to a global mix. This is an
undocumented system interface, so older-system and Intel runtime acceptance
remain required before claiming those configurations verified. No dependency,
permission expansion or persistently collected process identity is added.

## Acceptance on macOS 26.3.1, Apple silicon

All capture cases ran under the canonical signed development bundle, with
Screen & System Audio Recording disabled and System Audio Recording Only
enabled. Evidence contains counts and state only, never PCM or subtitles.

| Case | Result |
| --- | --- |
| QuickTime selected application, 16/24 kHz | Nonzero synthetic audio; stop drained, idle, zero post-stop buffers |
| Quiet QuickTime with outside playback | Zero audible samples at both rates; system-mix control nonzero |
| Empty helper inclusion list with outside playback | Zero audible samples; never expanded to global capture |
| Nested helper created late, stopped, restarted | Selected capture follows the helper at both rates |
| External WebKit audio helper | 91,381 / 136,500 audible samples at 16/24 kHz |
| Quiet WebKit host and a different WebKit host playing | Zero audible samples at both rates |
| WebKit helper created after capture starts | Inclusion count changes from 0 to 1; nonzero audio at both rates |
| 12 selected start/stop cycles and duplicate starts | All passed, idle/drained after every stop, zero post-stop buffers |
| Cancellation at 0/1/5/20/100/250 ms | All idle/drained with zero post-stop buffers |
| Selected application exits | ApplicationUnavailable, drained/idle, zero post-stop buffers |
| Missing selected application, then system capture | Unavailable with zero buffers; next start succeeds |
| Native permission error UI fixture | Localized reason, OS-settings action, retry returns to usable error state |
| Guide privacy action in seven languages | Explicit click invokes the fixed command; opener failure keeps a safe manual path |
| Native OS-settings action | Opens the correct privacy page; does not change either grant |
| Seven languages at the minimum settings width | Guide/error text and buttons fit; no horizontal overflow |
| Canonical check.sh | Rust fmt/clippy/tests and 131 frontend files / 2,039 tests pass |

The permission-error screenshot uses the credential-free UI-only fixture.
Actual denial/regrant was not repeated in this follow-up, to preserve the
existing grants while the owner is away. Existing broad-grant routing remains
covered by the selector matrix and prior v1.5.19 native acceptance; no new broad
grant was requested here. The capture proofs use generated tones and do not
claim live-provider speech recognition acceptance.

Earlier browser experiments were inconclusive: an agent-created tab can be
muted independently of its media element, and directly launching a WK test
binary gives it the terminal's responsibility instead of a normal app host.
The final two-app WebKit tests use LaunchServices and independent bundle IDs.
The initial zero-audio and backpressure results are retained in local evidence
and are not counted as passes.
