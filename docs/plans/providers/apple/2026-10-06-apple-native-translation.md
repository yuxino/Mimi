# Apple on-device text translation

Mimi offers Apple Translation as an independent text service for Apple Speech,
custom recognition and Alibaba recognition. It requires no API key, endpoint or
text proxy. Recognition credentials remain scoped to the recognition stage.
Selecting Apple does not copy, reveal or delete another translator's credentials.

## Language setup

Apple Speech uses one Recognition Language selector for both downloaded and
missing resources. The status and action always refer to that same selection.
Choosing a language only previews it. For a missing language, Download and use
prepares its resources and saves that language after readiness is confirmed;
for a ready language, Set recognition language saves it directly. An inactive
profile is activated together with its selected language. Failed preparation or
saving leaves the previous active selection intact and a retry beside the chosen
language. Resource preparation never starts audio capture. While subtitles are
running, selection and preparation stay disabled with the stop requirement visible.
The installation guide remains collapsed by default.

The text service displays the selected source and target language and checks
their actual native availability and installed-only session readiness. Models
that are not ready have an explicit download-or-enable action in Mimi. That
action opens an Apple-hosted translation session
with system consent and progress; completion rechecks the language pair. A
cancelled or failed operation leaves a clear retry path. Global model installation
alone does not prove readiness for a session. Recognition models and
translation models are separate. Already installed translation models are reused.
Help explains the in-app download flow.

Settings, tray and floating controls resolve the saved recognition language and
translation target from the same route capabilities. Settings can discover missing
Apple Speech resources; quick-switch controls offer only ready, compatible sources.
The target selector uses the existing field layout in settings and the same source-
dependent target list in tray and floating controls. A target switch is validated
before saving, including legal Tencent/Volcano directions. An unsupported target
is rejected rather than replaced with a different language. Paused sessions stay
paused; listening sessions reconnect with the saved source and target.

Saving a service choice does not trigger a download. Session start and connection
checks require installed models and fail before audio capture when models are
missing. They never open download prompts. Original-only and equal-language
passthrough keep their existing behavior. If recognition reports a different
source language, Apple translation uses that report before deciding passthrough;
an unknown report must not be committed as an already translated same-language
result. Translation requires an explicit source;
automatic recognition remains available for original-only where ASR supports it.

## Native boundary

The initial adapter supports Apple silicon on macOS 26 or later. Translation
itself exists on earlier macOS versions, but the installed-only session API used
to prevent unexpected download prompts requires macOS 26. The application keeps
its existing deployment target with runtime guards and weak framework loading.
Other platforms retain saved settings but do not advertise this service.

A Swift bridge owns Translation sessions and the explicit preparation panel on
the main actor. Rust uses bounded request IDs and callbacks; dropping a request
removes its callback and cancels native work. Native capacity remains occupied
until cancelled work actually finishes. Input, output and in-flight counts are
bounded. Errors cross the bridge as stable content-free labels.

The adapter reuses the existing high-quality scheduling path: replaceable preview
work, prioritized serial finals, shared final deadlines, and cancellation on stop
or route changes. The recognition transport and source identity remain separate.
Native supported languages are intersected with Mimi's encodable languages and
the recognition service's capabilities. Custom declaration stamps prevent stale
runtime snapshots from overriding an edited recognition language list.

## Verification

The Google source-language clarification is deliberately copy-only. Mimi's current
`gemini-3.5-live-translate-preview` integration sends an empty
`inputAudioTranscription` configuration and exposes automatic source detection.
The generic [Live API transcription configuration](https://ai.google.dev/api/live#AudioTranscriptionConfig)
does define optional `languageCodes` hints, but the current
[translation model guide](https://ai.google.dev/gemini-api/docs/live-api/live-translate)
does not establish their effect for this model. Help describes Mimi's current
integration, not a universal Google limitation. Target language remains selectable;
opening manual source choices requires model-specific validation first.

Cover credential isolation, native capability filtering, selected-pair probes,
original-only bypass, cancellation, failed preparation, stale UI results, and
native buffer/request bounds. Run the canonical repository checks. Inspect the
signed development app and record actual native translation and preparation
evidence separately from mocked UI layout fixtures. Android has no Apple API;
this intentional platform difference is documented in platform parity.

References: [Translation](https://developer.apple.com/documentation/translation),
[installed-only sessions](https://developer.apple.com/documentation/translation/translationsession/init(installedsource:target:)),
[download permission](https://developer.apple.com/documentation/translation/translationsession/canrequestdownloads).
