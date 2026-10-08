# Windows Live Captions input (#182)

## Product behavior

Add an experimental Windows Live Captions recognition profile. Recognition runs in
Windows; mimi reads the visible caption element through UI Automation and reuses
its existing bounded recognition/independent text translation/overlay pipeline.
This is not the Apple Speech SDK integration: Windows owns recognition, language
packs and the caption language. mimi cannot certify an installed language catalog
or request authoritative recognition final events from this accessibility surface.

- Require Windows 11 22H2 (build 22621) or newer and the system LiveCaptions binary.
- Open Live Captions only through an explicit user action. Guide first-run Windows
  consent, language-pack download, matching the caption language, and disabling
  “Include microphone audio”. Do not kill, hide or reposition its window.
- Require per-profile opt-in before reading captions. Consent includes confirming
  Windows microphone captioning is off. The accessibility interface does not offer
  a documented microphone-state API; this confirmation is not a runtime guarantee.
- Explain that the selected independent translator receives caption text. Original
  only performs no translation request. Local recognition does not certify local
  translation, and Copilot+ translation is not integrated by this change.
- Missing setup, a closed window, unsupported OS, an unreadable element and revoked
  consent have explicit feedback. An initialized Windows window can omit the
  caption element until speech arrives: guide audio playback and refresh before
  suggesting setup is incomplete. Never fall back to cloud recognition.
- This source represents Windows' mixed caption stream; it cannot certify per-app
  isolation, multiple independent sources or provide audio recordings. Reject
  incompatible capture/recording settings instead of misrepresenting them.

## Integration and lifecycle

Read only the `CaptionsTextBlock` inside a `LiveCaptionsDesktopWindow` belonging to
Windows' own LiveCaptions executable. UI Automation work runs on a dedicated COM
thread, with bounded snapshot delivery and text size limits. Caption text must
never appear in diagnostics or Debug output. Idle capability checks do not read
caption text; session reading requires consent.

Windows owns its recognition language. The settings recognition check must not
send the source-language override reserved for Apple Speech.
The overlay omits recognition API timing for this local source, while retaining
the independent translator's observed timing when translation is enabled.

Grant the support, explicit-open and consent IPC commands only through the
settings window's `app-settings` permission. Registering a Tauri command does not
grant the frontend access to it. Production permission tests must retain all three
commands and ensure other window permission scopes do not expose them; mocked
frontend support checks cannot validate the packaged application's ACL.

The first snapshot is a baseline, so existing screen text is not replayed into a
new session. Replaceable snapshots produce bounded previews. Stable text uses an
explicitly heuristic final boundary; Windows provides no public recognition-final
signal here. Clear, pause, stop, reconnect and generation changes retire callbacks
and baseline old visible content. Use the existing ordered translation lane and
bounded final queue, keeping credentials on the existing credential-storage path without adding a new fallback.

## Validation and limits

Run `scripts/check.sh` and Windows x64/ARM64 CI. Add tests for evolving snapshots,
rollover, corrections, repeated text, baseline suppression, bounds and lifecycle
cancellation, plus settings consent and frontend setup/error interactions.

Capture before/after settings evidence on Windows, using the PR base and head.
When explicit UI-test fixtures supply support states, those screenshots prove
Windows WebView interaction and layout, not live speech recognition or UIA
compatibility with a particular Windows language pack. Record real acceptance
separately: Windows build, actual language-pack installation, microphone state,
public video source, UIA behavior and translation-service timing. Keep evidence
provenance with the PR and do not mark unrun matrix entries as passed.

## References

- [Microsoft Live Captions setup, versions, languages and microphone behavior](https://support.microsoft.com/en-us/accessibility/windows/use-live-captions-to-better-understand-audio)
- [Microsoft UI Automation text model](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-understandingtheuiautomationtextobjectmodel)
