# Desktop settings: direct service selection and subtitle preview

## Decision

Extend the Android settings direction to the desktop surface while retaining multi-profile support. Keep the existing sidebar and category deep links. Use one clear content column: service rows first, an independent editor when needed, and a separate provider chooser for additions. Avoid a modal wizard, nested management disclosure, or duplicate editors for the same profile.

Configured profile rows activate directly while idle; the adjacent settings action opens its editor. Missing or unavailable credentials open configuration without starting capture. Saving credentials explicitly saves and activates the edited configuration, so first-time setup does not require a second Use action. Keep rename and deletion in secondary controls, preserve destructive confirmations, pending-action locks, the 20-profile limit, capability normalization and native failure reporting. Navigating away unmounts the write-only editor and discards plaintext drafts. No credential retrieval or storage changes.

The subtitle page starts with a clearly labeled local sample reflecting font size, alignment and immersive presentation. It never displays or retains live speech. Keep language controls prominent and group appearance controls underneath; fold less-used placement controls into a disclosure. General appearance, update and opt-in session export remain available. Preserve default-off capture/history retention and existing session behavior.

Use Mimi's neutral tokens, native typography, modest separators and existing artwork. Provide Chinese, English and Japanese copy and responsive keyboard-accessible controls. Do not change provider protocols or Linux integration.

## Verification and delivery

Run frontend and canonical checks, then use the signed canonical mimi-dev bundle for light/dark, empty, configured, error, paused, translating, collapsed and long-subtitle checks. Verify service selection, save/activate errors, discarded drafts and category state in the native WebView. Record a new 3840×2160 artifact from the current native build; clearly distinguish local sample UI from real provider subtitles. Keep private desktop content and credentials outside the frame. After design and video acceptance checks, refresh website discovery metadata, AI-readable product facts and GitHub topics using the actual Linux release state at that time.

## Verified implementation

- Canonical `scripts/check.sh`: 437 Rust tests passed, one ignored; 124 frontend tests passed, plus fmt, clippy, updater-manifest tests and production build. The existing SoftwareUpdate fast-refresh lint warning remains.
- Signed `/Applications/mimi-dev.app`, credential-free UI mode: subtitle preview alignment and size controls, light/dark appearance, service list, eight-provider chooser, Azure multi-field editor, navigation back, idle/listening states and disabled profile mutations during a session. The existing overlay control opens and returns to settings.
- Browser smoke: returning from an editor discards its unsaved test-only credential draft; Japanese source-language labels fit the 760px desktop width without horizontal overflow.
- Recording is a fresh native settings walkthrough in UI-test mode. Its sample subtitle is labeled in the product. It is not evidence of a new provider session or Windows/Linux native acceptance.
