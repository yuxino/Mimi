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
icon while the microphone is selected, either alone or with system audio. A
compact metadata line follows the subtitle alignment, with no left timestamp
gutter. At short heights, an icon without a timestamp moves beside the text so
retained mixed history cannot clip a single-source bilingual row.

Switching to system audio only hides the system source icon. With time disabled,
there is no metadata row. Retained microphone subtitles keep a small microphone
icon and their source color, so switching does not erase who spoke. Keep source-aware history
projection independent of this presentation choice. Single-source bilingual
originals remain neutral; merely showing a microphone icon does not tint them.
The primary subtitle lane uses its own source color even when only one input is
selected. Hiding a source label never discards its identity or color preference.

Restore an opt-in persisted “Show time” setting in subtitle appearance. Default
it off, including old settings/snapshots. For every selected input, display local
HH:mm:ss on confirmed subtitles; the stored timestamp is confirmation time,
not speech onset or a media
playhead. Do not invent a time for a streaming draft. The overlay control panel
and settings share the same opt-in, including Immersive Mode. Immersive metadata
uses near-white text, slightly larger icons/digits and compact black shadows; SVG
icons receive their own drop-shadow to improve contrast over light footage.
The switch takes effect immediately in both surfaces, including system-only
sessions. Preserve the preference and visible times through input changes.
Both single-input modes reserve a metadata row when time is enabled, raising the
minimum window height from 136px to 160px. This corrects the earlier microphone-only
condition, which left an enabled switch with no visible effect for system audio.
With time disabled, retained microphone icons stay inline. No transcript retention or
recording is enabled by this setting.

The floating control panel exposes both the time preference and pause/resume so
immersive users do not need to leave their current mode to control the session.
Pause/resume stays compact and right-aligned, is disabled during connection/stop
transitions or another pending panel action, and keeps the panel open on success.
The control window needs the same native pause/resume command permission as its
visible action; mocked frontend commands alone cannot verify this boundary.
Related actions share busy and sanitized failure handling. The drag handle keeps its
pointer-drag/double-click behavior and gains a tooltip, focus treatment and
Enter/Space activation, including narrow windows where the other collapse action
is hidden. Suppress native keyboard activation as well as repeats, composing
events and Alt/Ctrl/Meta combinations, so modifier release before Space does not
generate an unintended second click. Pending controls block dragging and collapse.
Browser preview corner resizing follows both axes for accurate layout
review; native resize ownership stays unchanged.

Android's expanded reading panel places optional history and the complete current
source/translation inside one bounded scrolling viewport. Current text remains
bounded by the shared subtitle core, without the compact view's two/three-line
display caps. Compact and immersive Android captions keep those existing caps;
this does not enable history retention or change capture/provider behavior.

Opening the expanded panel locates the start of the current caption. When its
content changes to a completed pair and the user is still reading the current
caption, the next layout locates the new caption's start. Draft updates keep the
scroll position; a user reading earlier history is not pulled back. The native
adapter reads the shared `displayPairFinal` flag rather than inferring completion
from text. Collapse and immersive-entry buttons keep single-line labels and size
their widths to their text, with the existing minimum touch width; the language
route uses the remaining space and can ellipsize. When the measured controls no
longer fit, use two rows: collapse/immersive actions above language/font controls,
without shrinking the system-scaled text. Re-evaluate this arrangement on
configuration changes while keeping the existing caption views and scroll area.

Reserve metadata space in line budgets and synchronized native/browser minimum
heights. Preserve history reading, alignment, text colors and live-tail behavior.
Debugger replay preserves each snapshot's selected inputs and timestamp
presentation. Verify settings persistence/failure feedback, seconds and source
semantics, small-window geometry, locale widths, native command permissions and
signed macOS UI-only states. Android checks must include long-caption endings,
replacement of a scrolled long sentence, retained history and deliberate history
reading. Record actual runs and remaining device/locale gaps in the integration
ledger; UI-only evidence never establishes hardware capture or provider acceptance.
