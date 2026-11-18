# Sentence dividers in floating controls

Expose the existing Sentence dividers preference in the floating control panel,
between early subtitle display and confirmation time. Reuse the settings label,
contextual help and shared toggle styling in Chinese, English and Japanese.

Both surfaces read `settings.showSubtitleDividers` and save through the existing
settings command. The backend broadcasts the committed settings snapshot to all
windows; the panel must not hold a second copy of this preference. External
updates, closing/reopening the panel and failed saves must preserve that shared
state. Failures use the panel's existing sanitized feedback and allow retry.

Keep the existing default off. Immersive Mode hides the lines without clearing
the preference, so leaving that mode restores them. No capture, translation,
recording or Android behavior changes. The tray has no separate divider toggle.

Verify localized help, successful saves, failed-save retention/retry, broadcast
updates and immersive-mode preservation. Inspect the expanded native panel and
both directions of synchronization with Settings in the signed UI-only app.
