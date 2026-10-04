# macOS capture sample-rate ABI correction

ScreenCaptureKit declares `SCStreamConfiguration.sampleRate` as `NSInteger`.
The pinned `screen-capture-kit` 0.7.1 wrapper declares both accessors as `f64`.
Mimi called that setter when configuring its system-audio stream. On arm64,
integer and floating-point arguments use different registers, so the call does
not supply the intended provider rate to the native method.

A configuration-only native probe, without starting capture or accessing
credentials, confirmed the runtime signatures accept `isize` and reject `f64`.
Reading the property with the correct integer getter after the wrapper's 16000
setter returned an unrelated large integer. Calling the integer setter returned
16000 and 24000 as requested. The particular invalid value is not deterministic
and must not become a regression expectation.

Use a small macOS adapter helper that sends `setSampleRate:` with `isize`. Keep
the current provider rate, mono channel selection, capture filter and actual
ASBD-based resampling. Do not change the dependency or its cached source. The
focused regression constructs the native configuration object and calls the
production helper for both supported rates, then reads the integer property.
It does not start a stream or request recording permission.

The concurrent anime investigation recorded entirely zero sent PCM with both
single-application and all-application capture. Those cases cannot assess ASR
or translation quality. The ABI defect is independently established; that
configuration-only proof does not establish the cause of the zero PCM. Native
same-source capture must be retested with the signed development app and its
actual packet format checked separately.
