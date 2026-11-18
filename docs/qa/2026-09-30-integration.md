# Stable feature integration — 2026-09-30

This is one review and acceptance entry point for today's stable changes.

Latest cloud Linux QA at exact `197f165` reports: DeepLX bad endpoint retains
draft and nearby focused error; localhost endpoint with empty token saves using
the in-memory ASR key, and Follow restores the route. Synthetic subtitles and
pause/resume pass. Export cancel/save/failure were blocked by UI-only archive
clearing on stop, not established as a production defect. The minimal opt-in
[`MIMI_UI_TEST_EXPORT=1` fixture](../plans/development/verification/2026-09-30-ui-test-export-fixture.md)
addresses only that acceptance entrance; native dialog results on its exact
new package are pending. Seven-phase/streaming fixture coverage remains absent.

Follow-up cloud QA at exact `38bc4dfb065827ef8f1f8b365029343b43d719a0`
verified artifact11107067102 and its zip SHA256
`dd7781f7a27fe0a7c0242546d070a11e57b7cb1296f4af8411ac45c902181173`.
Stopped-state synthetic archive retention, native save-dialog cancel with
retained/re-exportable content, actual bilingual timestamped TXT save, invalid
path feedback and corrected-path retry all passed; both saved files matched.
An invalid file-as-directory is native picker/path feedback, not evidence of
backend disk-write failure. Backend disk-write failure, seven phases/streaming
and real provider/audio remain untested. The isolated app exited normally.

The latest user visual feedback asks for a sound-like pulse indicator. The
published continuous-ring video remains the previous candidate for comparison,
not final visual acceptance. A waveform candidate is being reviewed separately;
all other integrated features retain this same PR acceptance entrance.

The user then explicitly chose to keep both sound directions from `d72d5be`,
with a persisted selector and recoverable classic light. This supersedes the
source preview note suggesting selection of just one. Integration adapts the
source shapes without importing the preview branch history, keeps classic as
the migration default, and applies existing independent motion settings to
preview/overlay. See [implementation and new acceptance boundaries](../plans/desktop/overlay/2026-09-30-selectable-sound-pulse.md).
The diagnostic help note moves into optional details. Android and held desktop
guide copy is shorter in all three languages while true first-caption proof
stays unchanged; new exact-head screenshots remain pending. The existing
onboarding screenshots are previous-copy evidence, not the new after state.
Desktop copy-only PR85 follow-up is `4177c48a05c46c842ff98372c37f3df64a28ed23`,
still held outside integration; three-screen art/character redesign is absent.
Desktop onboarding #85 is on hold after user visual feedback. PR #67 is included
at the user's request, preserving the contributor's sentence blocks and text motion.
Only separator/timestamp/phase hunks from the existing `113a340` candidate are
adapted for review; the unapproved font/layout preview remains outside scope.

The user watched the continuous-phase video and approved publication and
inclusion as an integration candidate. Source `3fb4665` is independently
cherry-picked as `259bce1`, with no old task15 history or unrelated changes.
Its reference indicator is exact `42f7583`; DeepLX `fcdc1a9` uses the same
PulseRing/Timeline/index.css. This is still pending installed-package/native
acceptance; there is no merge or release authorization.

## Source and delivery ledger

| Source | Exact head | Integration state |
| --- | --- | --- |
| #72 sound source, capture status, diagnostics and toast | `4a8e5502a06f66a1d04bd78310ff8afd35731f32` | Already merged as `7cbb763470be28816fb03b26a4e4b712ea0a98d7` |
| #73 DeepLX and credential-form correction | `8437b9be385a85877f341b3b5e4d9b740b0856c8` | Already merged as `5c9cb685fb3c9ad7e20626f20d0032209a0054e6` |
| DeepLX advanced selection follow-up / #70, #73 | `93442f07a277082ec603019f2f38b80b504fe1ff` (`482da0f` behavior) | Included; default follows service, advanced Alibaba Audio3 + DeepLX only |
| #76 Android capture observations | `2471629c4a8917a16179e71633fa4b33f7b4f413` | Already merged as `d190877a6d3dfa3ca13f4ee05466b234e7ed26ed` |
| #67 sentence blocks, text motion and independent preferences | `3128c46c0c594aaa1cd899c2dd5630182ac3154c` | Included via merge `15b346b`; source PR remains open |
| Continuous phase candidate / #87 | source `3fb46654e7a3d4f053462e71c61ad5b6987f6d2e`, integration `259bce16902b48f73ae1debc404d94ad51908259` | Candidate only; user visual confirmation, native/Linux pending |
| #84 history export/deletion interaction | `42e2553990fd85f14cd74058818725a0c607d789` | Included here; source PR remains open |
| #86 Android first-run guide | `1e7de491646929871cfd1854f46b1ce3dd3c7c6a` | Included here above #76; source PR remains open |

The first three merges happened under the original serial-merge authorization,
before the user requested a single integration PR. This PR starts at the live
main `d190877`; it does not repeat, revert or pretend those merges are pending.
Do not merge the remaining source PRs independently during integration review.

## Public before/after evidence and regression steps

| Source / issue | Public evidence | What it establishes |
| --- | --- | --- |
| #67 / #87 | [Public original contributor captures](https://github.com/yuxino/mimi/pull/67#issuecomment-5888110417), [existing three-version comparison](https://github.com/yuxino/mimi/blob/6d4206648eab09c7b0523dba0ce8b5fc8194a407/README.md) | Contributor Windows source captures, plus a separate real React/CSS browser preview with identical synthetic text/seven phases. Rows use #67 Timeline, local `a7b6297`, and its `113a340` candidate with shared local support modules. Neither later row is exact main/#88; visual/native acceptance pending. |
| #72 / #74, #75 | [Native toast before/after](https://github.com/yuxino/Mimi/pull/72#issuecomment-6398718962), [checklist](windows-audio-source.md) | macOS 1.5.5 UI-only, Chinese/light, 760×720 logical at 2×; actual prior PR `2ec1b92` versus final toast build. No Windows hardware or paid ASR claim. |
| #73 / #70 | [Linux form before/after and steps](https://github.com/yuxino/mimi/blob/e98f352e8e7699695ec8ba5d12bc7880310e591e/docs/qa/pr73/README.md), [PR evidence](https://github.com/yuxino/mimi/pull/73#issuecomment-5908237856) | Native AppImage, English, 1180×812; `367013c` versus `8437b9b`; synthetic Update Credentials invalid-endpoint case. No private-host authentication/live translation. |
| #76 / #78 | [API 35 before/after](https://github.com/yuxino/mimi/pull/76#issuecomment-5910539044), [issue evidence](https://github.com/yuxino/mimi/issues/78#issuecomment-5910548336) | Actual Pixel 7 emulator, 1080×2400; `5f2a595` versus `2471629`, idle home/diagnostic entry. No capture/provider session or physical route proof. |
| #84 / #77 | [History before/after](https://github.com/yuxino/Mimi/pull/84#issuecomment-6398719875), [executable cases](history-interaction.md) | Real React component with mocked IPC, ego-lite Chromium, English/light, 1000×1100, UTC; `5f2a595` versus implementation `01b01af`. Does not prove native save dialog or filesystem behavior. |
| #86 / #82 | [Phone guide before/after](https://github.com/yuxino/mimi/pull/86#issuecomment-5910538568), [issue evidence](https://github.com/yuxino/mimi/issues/82#issuecomment-5910547866), [versions, APK hashes and commands](https://github.com/yuxino/mimi/blob/f1ca9db23532d4c5ec5c8a1ee5b0eb0242791a0e/README.md) | Actual API 35 Pixel 7, 1080×2400; baseline `2471629`, 48 Chinese/Japanese light and English dark guide screenshots at `b847050`; final `1e7de49` changes only xAI pricing URL. No live caption or physical-device claim. |

### Continuous-phase candidate — new evidence

[Fixed public original image/video and provenance](https://github.com/yuxino/mimi/blob/e7a1a38a6a5b9c956dcd8bac987049134a5d9191/README.md).
Actual Mac Chromium component capture, exact reference indicator `42f7583`
versus candidate `3fb4665`, with identical baseline Timeline/style/font and
synthetic bilingual content/seven phase inputs. No native window, real audio
or Linux installed package is shown. Image SHA256
`fc5d44ec266167c08ab245a04b58f5f7e9a01c7ff61ba25b7ed9f5351d5f3aa9`;
video SHA256 `958df845ab52994ff0a2e3bc7d193312abf0b014b647031b5c79852e7d919a11`.
H.264 1440×1032 MP4, 20.445s, variable original screencast timestamps; 1016
recorded frames, 17.556s changing-frame span at 57.814fps average. Static gaps
and the final 2.887s remain; no interpolation or speed change, no fixed/native
60fps claim. Source implementer and user watched the actual video.

Only PulseRing and component CSS change. Seven clocks persist through active
phase crossfades; pause/error/idle settle for 520ms then pause, resume continues;
resolved motion off is immediate. Existing #67 text/sentence motion, two
independent preferences, 18/40px boxes, settings, capture and compact labels
remain. Source typecheck/component lint and browser identity/pause/resume/off
checks passed; exact aggregate CI is a separate requirement.

User explicitly approved posting these materials and adding the candidate to
#88 for acceptance. The older three-row local-baseline material stays historical
and must not stand in for this new exact-indicator comparison.

Linux follow-up requires the exact integration commit/artifact SHA256, isolated
credential-free UI-only data, same-size screenshots and observed state sequence
in the actual installed app. Current nativeGUI/install freeze still applies;
no launch/install is attempted here or hidden behind a Linux test route.
Signed WebKit, Linux installed UI, real service/audio/physical routes remain
untested. Do not claim CI bundle compilation proves them.

### DeepLX advanced interaction follow-up

Normal Alibaba setup remains one key, with advanced text translation following
the service by default. Only the already-implemented Alibaba Audio3 + DeepLX
chain can be selected separately. Historical profile IDs/provider accounts
remain unchanged. Ordinary Alibaba keys remain raw in their existing secure
items; the new endpoint/token use a separate profile-scoped secure destination,
and the ASR key is not duplicated or returned to the WebView. Route switches,
metadata and secure-item rollback are covered with synthetic stores.

[Source handoff and verification boundary](https://github.com/yuxino/mimi/pull/88#issuecomment-5912410350).
The source reported 224 frontend tests, typecheck/lint, 40 core tests and six
selected fake-store transactions; those isolated native probes do not establish
full app/keychain acceptance. Integration review restored the test-only
`load_api_key` helper still used by old migration/credential tests, so exact-head
remote CI can compile and exercise the complete existing test module.

The earlier #73 Linux pair above only proves invalid-endpoint feedback; it does
not show this advanced layout. New matching screenshots and signed native
acceptance remain pending. When coordinated, check ordinary one-key setup,
advanced DeepLX with no repeated key, invalid endpoint draft/focus, unsupported
service choices absent, historical profile selection, route switching back,
and isolated write/delete rollback. Do not use real credentials or providers
to generate UI evidence while native access remains frozen.

### External Windows feedback

[LLLin000 reported Windows 11 observations on upstream #72](https://github.com/yuxino/mimi/issues/74#issuecomment-5912332669):
Realtek to ToDesk Virtual Audio followed during runtime, with received-sound
status and no reconnect/error; stopping the source showed no recent audio data.
No exact SHA was supplied. This is contributor self-report, not independently
retested or integrated-package evidence, and does not establish all three sound
states, live captions, Teams/Bluetooth/hotplug acceptance. #74/#78 remain open.
Role routing, silence billing, reconnect and per-app-capture suggestions need
separate source/official verification and are not added to #88.

All linked assets are safe public repository images with version/fixture
boundaries. Library/private file URLs are not acceptance evidence. These are
source-PR results; an integrated native screenshot is still pending.

## Integrated checks and native pause

After the user explicitly added #67, its six unabsorbed commits merged without
textual conflicts. No whole local preview branch is imported. The adapted
`113a340` hunks keep #72 compact labels/source geometry and all DeepLX/Android
settings intact. Independent regression `6ffdbff` fixes translation-only history
disappearing when original and translated text are equal. The card Timeline
receives its resolved motion preference and smooth new-block scrolling obeys it.

Exact head `42f758378e0d2b47bc65922ac105f0907f5a62f1` passed ten applicable
[desktop/cross-platform](https://github.com/yuxino/mimi/actions/runs/36715331676)
and [Android](https://github.com/yuxino/mimi/actions/runs/36715331394) jobs,
including 214 frontend tests. These are pre-advanced-DeepLX results.
The green local results below belong to the earlier **pre-#67** tree. Exact new-head
remote CI must pass before acceptance. No local build, installation or launch
was restarted to validate the expanded scope.


Local merge `71ac33519f311f2e22782093e3e66397143310a9` has the same Git tree
`10c7b3c157bd829b88e4751ecc8c5120d56d36cb` as the tested trial merge.
`./scripts/check.sh` passed: 509 Rust tests, one ignored; 197 frontend tests
across 28 files; fmt, strict clippy, safety scripts, updater tests, lint,
typecheck, icon verification and production frontend build. The existing
SoftwareUpdate Fast Refresh warning remains, with zero lint errors.

Shared commands/settings/types and provider state merged without textual
conflicts. The aggregate checks include provider secure-storage isolation,
DeepLX ordered/cancellable loopback requests, diagnostics privacy, toast
lifetime/navigation, and 16 history interaction regressions. Final PR CI is a
separate requirement from source-head CI.

The additional local Android rerun did not pass: its fresh offline Gradle
cache could not resolve Android plugin 8.7.3. The already-green exact #86
Android CI and source debug/release 39-test/lint/build evidence remain valid;
this failed local rerun is not new acceptance. No cache-repair rebuild was
started after the user requested lower machine load.

Native install, signing and launch are paused after the repeated Keychain
prompt report. A UI tool initially auto-launched the old normal development
bundle; it was immediately quit. It was not the integration bundle or an
UI-only launch. The integration build had not reached installation/signing;
its task-owned compilation and installation processes have been stopped.
The prompt requester could not be confirmed by the bounded read-only GUI
observation. No Allow/Always Allow, password, ACL change or credential reset
was performed. Do not count the old bundle as integrated native QA.

Resume only through the one integration coordinator, using one fixed
development path and signing identity after the Keychain handoff is arranged.
First run credential-free native settings/panel/overlay plus synthetic TXT
export cancellation/save regression. Keep saved-history fault injection
isolated; never seed/delete ordinary user history. Live-provider and physical
hardware routes stay separate from these UI checks.

## Issue closure rules

Authors were queried live on 2026-09-30. Only an issue opened by `yuxino` may
be closed automatically, and only after the integration is merged and that
issue's acceptance is actually resolved. Leave a PR/evidence comment at close.
Use references here, without broad automatic closing keywords.

| Issue | Author | Closure decision / remaining condition |
| --- | --- | --- |
| #70 custom API | `294910541` | External author: do not auto-close. General gateway/custom ASR/private compatibility remain outside this partial DeepLX implementation. |
| #74 Windows audio source | `yuxino` | Keep open: real Windows headset/Teams, default/manual routing, hotplug and live subtitles untested. |
| #75 safe diagnostics | `yuxino` | Keep open: source native copy/privacy evidence exists; complete integrated preview/copy and the requested credential-state evidence before closing. Unknown internals must remain unknown. |
| #77 history interaction | `yuxino` | Candidate to close after merged integration and native synthetic export acceptance; component regressions pass, native cancellation/save remains pending. |
| #78 cross-platform capture | `yuxino` | Keep open: Android 10/14/15 physical capture, Bluetooth, macOS routing/TCC and Linux physical PulseAudio/PipeWire routes remain untested. |
| #82 first run | `yuxino` | Keep open: desktop #85 held for user visual review; live first-caption and relevant platform acceptance are incomplete. Android #86 is reviewed independently. |
| #87 subtitle detail review | `yuxino` | Keep open: continuous-phase visual candidate accepted for integration review; exact installed/native, reduced-motion and Linux acceptance pending. |
| #83 repeated Keychain prompts | `yuxino` | Keep open: actual reinstall/authorization root cause is unresolved. Local `a9858e8` improves safe classification only; it is not included or claimed as an authentication fix. |

The minimal local font commit `8243b24` remains outside this integration:
its default stays the system font and its tests cover choice/persistence,
but native experience is pending. The broader `54671f5` visual preview has
not received approval and is not enabled by this PR. #67 is included and its
original contributor design remains the baseline. Issues #48 (`yeshuoer`),
#59 and #61 (`LLLin000`) are already closed and externally authored; this
integration does not change their state. #67/#84/#86 source PRs remain open
until integration acceptance and a separately reported duplicate disposition.

No tag, release, production deployment or formal-app replacement is authorized
by this PR. A ready source check or an APK/bundle build is not a release.
