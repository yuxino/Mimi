# Microphone controls and subtitle time

Restore the desktop microphone capability after the temporary system-only release.
System audio remains the default; microphone selection remains explicit. Keep
independent capture, source identity, recording consent reset, and existing error
handling. Android provider/capture behavior is unchanged.

The floating control panel groups the two independently selectable sources in one
compact row, with application selection below. Help stays beside the group label;
source observations and device details remain available per source. Switching
keeps the existing lifecycle and last-source guards. The default system source
no longer adds an icon to the compact status capsule; its accessible description
and the expanded source choice remain. Microphone-enabled modes add one mic icon.

Replace the large source-name column beside every subtitle with a small source
icon. Mixed-source history retains both identities after changing inputs. A
compact metadata line follows the subtitle alignment, with no left timestamp
gutter. At short heights, an icon without a timestamp moves beside the text so
retained mixed history cannot clip a single-source bilingual row.

Restore an opt-in persisted “Show time” setting in subtitle appearance. Default
it off, including old settings/snapshots. Display local HH:mm:ss on confirmed
subtitles; the stored timestamp is confirmation time, not speech onset or a media
playhead. Do not invent a time for a streaming draft. Immersive mode still hides
timestamps. No transcript retention or recording is enabled by this setting.

Reserve metadata space in line budgets and synchronized native/browser minimum
heights. Preserve history reading, alignment, text colors and live-tail behavior.
Debugger replay preserves the chosen timestamp presentation. Verification covers
settings persistence/failure feedback, seconds and source semantics, small-window
geometry, all three locales, and signed macOS UI-only states. UI-only evidence
never claims microphone hardware or live provider validation.
