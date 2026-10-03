# Subtitle background opacity and readable history

The desktop subtitle card defaults to 80% background opacity instead of the previous
62% expanded / 68% collapsed fill. A Background transparency slider in Subtitles
exposes 0–100% transparency (default 20%). It saves immediately, updates the sample
and both overlay card sizes, and does not change text alpha or restart capture.

Persist `subtitle_background_opacity` as a bounded integer percent in global
preferences; send `subtitleBackgroundOpacity` through settings snapshots/drafts.
Missing legacy values default to 80. Clamp values above 100 on load and save.
Use the same fill on macOS, Windows and Linux; remove Linux's fixed-alpha override
and decorative gradients so fully transparent means no background fill.

Immersive mode still hides the fill. Disable its transparency slider while active,
preserve the chosen value, and restore that value when leaving immersive mode.
Historical utterances retain the same text contrast as the latest one: remove the
age-based block opacity and timestamp dimming. Preserve bounded history, scroll
behavior, source reference styling and existing entry motion.

Verify default migration, persistence/bounds, IPC serialization, active-session
visual updates, transparency direction and preview, both overlay sizes, immersive
restoration and history contrast. Run the repository checks and signed UI-only app.
This is a desktop presentation change; Android's native subtitle styling is unchanged.
