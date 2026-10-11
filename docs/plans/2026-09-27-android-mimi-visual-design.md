# Mimi Android visual design

Reuse Mimi's actual desktop identity: the existing pink character artwork and
lowercase wordmark, surrounded by the restrained grayscale UI from
`src/windows/settings/settings.css`.
Keep the original character pixels intact. Adapt them as an Android adaptive
launcher foreground, with a separate caption glyph for monochrome notifications.

Use a system-following light/dark palette, native system typography, 24 dp side
margins, thin separators, restrained 10–14 dp corner radii, and a single high
contrast primary action. No teal product theme, gradients or decorative cards.
The dark subtitle preview is a functional example of the real floating subtitle
surface and explicitly labels its text as sample content.

The home screen presents current languages and provider configuration above a
fixed session action. Missing credentials route the primary action to service
configuration and explain the user's API key requirement inline. Normal refreshes
must not emit repeated setup toasts.

Settings has two tabs: translation service and subtitle appearance. All controls
stay inflated while switching tabs so provider drafts and unsaved edits survive.
Optional endpoint/model fields are collapsed. Appearance shows a sample that
responds to font, color, opacity and background changes. Existing saved color
choices remain intact; new installations default to white subtitles.

Validation uses Android unit tests, lint, APK build and installation in the local
API 35 emulator, then checks light/dark layouts and settings interactions. These
UI checks do not prove real provider translation or physical-device audio capture.
No upstream push or release is authorized for this preview.

## Fewer-step refinement

The home language summaries are now direct selection sheets. A selection saves
immediately, updates the sample, and offers Undo; running sessions retain their
current languages until restarted. Tapping the subtitle sample opens appearance
settings directly. Appearance edits save on user interaction, while provider
credentials and advanced service drafts still require an explicit Save.

Settings prioritizes the preview, font size, background and color. Less-used
opacity/history controls live under More options; glossary and custom endpoints
live under Advanced. History remains explicitly opt-in. Permission setup resumes
the user's pending start after granting overlay access, and the optional
notification prompt is deferred until a session is started.

Demonstration footage is a real emulator screen recording of the installed APK.
It uses the labeled sample only and never initiates audio capture or a provider
session. UI tests restore changed non-secret preferences after the demonstration;
credentials are neither edited nor exported.

Instrumentation must synchronously flush restored non-secret preferences before
finishing: Android terminates the instrumented process immediately, which can
otherwise interrupt asynchronous preference writes. Cold-process readback is
used to verify restoration before recording another walkthrough.
