# Current design records

This directory keeps decisions that still constrain the application. Start with
the [documentation index](../README.md) and current development guides for setup,
commands and review rules. The links below are a map of major decisions, not a
second setup guide or a complete list of every protocol record.

Completed task checklists and superseded designs belong in Git history. Promote
reusable rules into their owning guide instead of keeping duplicate audit notes.
Dated comparisons and QA/release reports describe the revision and conditions
actually tested; they are not ongoing authorization to capture audio, contact a
provider, merge or publish a later change.

## Storage, development and distribution

- [Private credential files and one-time OS-store import](2026-10-04-local-credential-storage.md)
  — durable writes, profile isolation, import checkpoints and verified retirement.
- [Independent development profiles without `.env`](2026-10-08-remove-dev-env-presets.md)
  — separate editable data and explicit copying from a formal installation.
- [Session history](2026-09-28-session-history-design.md)
  — default-off, bounded local transcript/audio retention, deletion and export.
- [Stable release signing](2026-09-26-stable-release-signing-design.md)
  and [recording permission continuity](2026-10-04-recording-permission-continuity.md)
  — stable installed identities and separate recording grants.
- [Build throughput](2026-10-08-build-throughput.md)
  — local validation/package profiles, cache boundaries and updater verification.
- [Windows portable releases](2026-09-26-windows-portable-release-design.md),
  [Linux desktop support](2026-09-27-linux-support-design.md) and
  [Android release signing](2026-09-27-android-release-signing.md).
- [Signed in-app updater](2026-09-02-signed-in-app-updater-design.md)
  — user-initiated download/install/relaunch and release metadata.

## Streaming and audio

- [Subtitle quality training with existing services](2026-10-10-subtitle-quality-training.md)
  — fixed samples, accuracy and timing evidence, and native reading acceptance.
- [Shared subtitle core](2026-10-03-shared-subtitle-core.md)
  — desktop/Android pairing and finalization ownership.
- [Runtime reliability](2026-08-24-runtime-reliability-and-release-design.md)
  and [audio/HTTP lifecycle](2026-09-07-audio-http-lifecycle-design.md)
  — bounded queues, cancellation, recovery and generation ownership.
- [Transcript publication boundaries](2026-09-08-transcript-publication-boundaries-design.md)
  and [final request identity](2026-10-04-final-request-identity.md)
  — draft/final separation and stale-response rejection.
- [Live profile switching](2026-10-05-live-profile-switch.md)
  — session replacement, rollback and selection guards.
- [Optional microphone input](2026-10-03-optional-microphone-input.md),
  [application audio](2026-10-03-application-audio.md) and
  [Windows output selection](2026-09-30-windows-sound-source.md).

## Service and language contracts

- [Custom speech recognition](2026-10-03-custom-speech-recognition.md)
  and [recognition/translation settings](2026-10-03-service-stage-settings.md)
  — independent stages, credentials and explicit checks.
- [Provider language catalogs](2026-10-06-provider-language-catalogs.md)
  and [per-profile language presets](2026-10-07-profile-language-presets.md).
- [Gemini setup and protocol evidence](2026-10-04-gemini-live-translation.md)
  and [Volcano subtitle protocol](2026-10-07-volcano-subtitle-protocol.md).
- [Apple Speech](2026-10-05-apple-speech-design.md),
  [Apple Translation](2026-10-06-apple-native-translation.md) and
  [native language packs](2026-10-06-apple-language-packs.md).
- [Alibaba translation model selection](2026-10-07-alibaba-translation-model-selection.md).

## Product surfaces

The [UI guidelines](../development/ui-guidelines.md) own shared layout, editable
fields, automatic saves, feedback and cross-page verification rules. Keep
surface-specific behavior in the relevant record:

- [Subtitle overlay](2026-08-23-overlay-behavior-design.md),
  [background opacity](2026-10-03-subtitle-background-opacity.md) and
  [overlay close control](2026-10-07-overlay-close-button.md).
- [Settings appearance](2026-09-24-settings-appearance-design.md)
  and [explicit credential reveal](2026-10-02-explicit-credential-reveal.md).
- [Interactive pointer behavior](2026-10-03-interactive-pointer.md)
  and [fullscreen Space following](2026-08-26-fullscreen-space-following-design.md).
- [Android subtitle interaction](2026-10-07-android-overlay-interaction.md)
  and [interface languages](2026-10-07-android-interface-language.md).

When behavior changes, update its owning decision and guide. Keep historical
test counts and candidate-specific instructions out of current setup guidance.
