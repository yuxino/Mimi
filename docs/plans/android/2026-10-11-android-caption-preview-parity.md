# Android caption preview parity

The Captions preview must show the actual floating caption frame. The user
rejected a separate sample card whose layout, wrapping and controls differ from
the real window, and requested the desktop subtitle background.

Extract the native normal panel and immersive caption views into shared Android
rendering components. Both the service and Captions preview instantiate these
components with the same typography, chrome, colors, surface, display mode and
alignment. Capture, window placement, service lifecycle and subtitle policy stay
in their existing adapters. Preview chrome is decorative and cannot start a
session, resize a system window or accidentally navigate away from its controls.

The preview measures a complete frame at the real overlay width and height
against the display's safe viewport. Fit that frame uniformly into a stable
preview stage, preserving line wrapping and proportions. Settings remain pinned
and independently scrollable; font, width and height edits must not move the
slider being dragged. While following, normal mode allocates measured visual
lines to both languages from its actual reading height. Short text gives unused
space to the other lane, and long text follows its newest complete lines. A
deliberate upward scroll opens the complete bounded text for reading; returning
to the bottom resumes following. Keep the user's window height stable as history
grows, with a minimum for one complete original line and two translation lines
in bilingual mode where the safe display permits, using actual fallback-font
tail metrics. Reserve optional time even while a draft hides its metadata. Immersion
retains the real compact caption line limits.

The new-install normal panel defaults to 32% of the usable display height. This
leaves comfortable bilingual reading space without occupying most of a portrait
screen. Keep explicit saved heights, direct resizing and the 20–70% height
control; actual glyph/chrome minimums and the safe display cap still apply.

Use pure black and an 80% default background, matching desktop's
`subtitleBackgroundColor`. The separate Android whole-window opacity defaults
to 100% so default translucency is not multiplied twice. Preserve explicit
saved opacity preferences and expose background 0–100%. Preview and live frame
use the same surface function from the first frame onwards.

Native acceptance compares both renderers at the same measured size and content,
including shape, chrome, wrapping, typography and surface pixels. Cover normal,
immersive, waiting, error, long content, all display modes/alignment choices,
both provider identities, width/height/font/alpha endpoints, all eight locales
and narrow/landscape large-font layouts. Retain direct live-style, movement,
resize, history/immersion and own-page hiding regressions. Test fixtures must
remain capture/provider-free and restore the development app's private settings.

The transparent immersive preview uses a neutral dark stage as a video-like
reference surface so white text stays visible on the light settings page. This
is outside the shared caption frame and does not change its transparent paint.

The floating quick editor also previews the complete frame. Keep the existing
live window, route, drag mark, provider identities, action appearances and current
subtitle/status/history; opening the editor must not hide its toolbar or footer.
Temporarily intercept preview gestures and remove action accessibility/focus
without dimming the controls. Closing or failing to open the editor restores
normal interactions on the same views. This applies to the quick entry as well
as the primary Captions section; checking only one entry cannot establish parity.

While the quick editor is open, the immersive restore companion must clear the
editor's occupied bounds and have a native touch-through window. Disabling a
View's click listener alone can still consume taps intended for an underlying
switch. Dismissal restores the native touch target along with ordinary actions.

Measure the live panel at its ordinary saved width/height while editing. If the
editor leaves too little room, scale that complete drawing uniformly above it
rather than reducing its reading viewport or clipping chrome. Do not persist
temporary scaling or positioning. Cover live waiting/error/content updates,
inert home/service/style/immersion/drag/resize gestures, restored actions, both
provider badge cases and narrow/landscape large-font layouts in native checks.

Sentence dividers and confirmation time are independent appearance switches,
both off by default, available in primary and quick settings. Dividers belong
between sentences and remain hidden in immersion. Time uses shared-core epoch
milliseconds from an accepted final, formatted locally as HH:mm:ss, including
with history disabled; never assign a Kotlin arrival time or label drafts as
confirmed. Show the same metadata in the real and preview frames, and reserve
its measured height before allocating bilingual lines. Android remains playback
capture only; these settings do not add a microphone source.

Continuous updates must preserve the native tail viewport after TextView's own
pre-draw scroll reconciliation, and must never cancel a window draw to follow
text. Read source/translation/metadata atomically from one published shared
state, coalesce redundant pending paints, and skip identical text assignments
on health/status publications. Reserve a
stable multilingual fallback-font minimum from the chosen typography rather
than recalculating the window height from each incoming word. The chosen font
size applies directly to the translation and original-only text; bilingual
originals use a 90% reference size and translations use medium weight, without
the previous extra 3 sp/bold boost. Use the same typography in all previews and
normal/immersive frames. Rotation cancels transient resize geometry, retains
saved width/height and positions, and clamps the visible frame to safe bounds;
returning to portrait must recover the original size. Ignore remaining events
from a gesture started in the previous configuration, including its final up.
Check continuous drawing
with eight language fixtures, all three modes, height endpoints and actual orientation
round trips, then rerun comparable real playback.

Realtime services can publish the next original before its translation. Keep
the shared reducer's retained confirmed pair visible in translated reading modes
until a new translation exists, even with history disabled. Select the whole
pair and its confirmation metadata; never combine its translation with the new
original. Original-only mode continues to show current recognition. This is a
native frame selection from the existing shared snapshot, with no extra retained
transcript, reducer, provider policy or timer. Clear/hidden frames must not revive
old content, and a fresh session with no confirmed pair still shows listening.

Original reading selects the shared raw recognition lane even when the service
holds complete translated pairs. Translation delays must not delay that view.
Show confirmation time only when the raw source is final and belongs to the
shared confirmed pair; a different owner's time cannot accompany a new draft.
If ASR opens its next utterance with an empty draft, keep the shared completed
original and its own time until actual new words arrive. An empty recognition
handoff must not flash the listening placeholder. This selects existing shared
content, with no additional transcript retention; hidden/clear frames and a
fresh session still remain empty. A complete shared preview may likewise stay
visible across an empty opening draft, but remains unconfirmed without a time.
Exercise both atomic and realtime projections
through JNI, native previews and real original-only playback handoffs.

For #336, a pause between realtime updates keeps an already visible translated
draft readable even before the first confirmation. The idle timer still applies
to an unfinished source alone. If the service retires an atomic preview, the
frame can use the latest confirmed row retained for that input, with its own
timestamp, until the next pair arrives. An empty history provides no fallback.
This reading choice neither preserves withdrawn drafts nor adds history rows.

Keep realtime intermediate captions without adding a reading-speed switch or
filtering drafts. Normal-mode text follows the desktop bottom reading edge:
short current content sits above the footer and grows upward. Increasing the
original's line count must not move an unchanged translation downward. Preserve
the actual glyph-based lane allocation and deliberate history scrolling; do not
reserve empty fixed-height language lanes or change the saved window height.
Verify native baseline positions while one language grows, across all three
modes, as well as preview parity and a comparable continuous live sample. This
placement does not prevent a service from revising an incomplete translation;
reader comfort and semantic revisions require separate observation.
