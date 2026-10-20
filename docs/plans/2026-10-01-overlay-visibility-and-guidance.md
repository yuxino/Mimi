# Live subtitle feedback: visibility, space, controls and timings

The user requested clearer status, a larger empty-overlay indicator, less
wasted subtitle space, bounded long-sentence presentation, simpler controls,
a discoverable guide, meaningful latency readings, and quicker reactions to
short recognition drafts. Work branches from merged integration #88 and is
reviewed through a new PR; this iteration publishes no version.
The branch also integrates main's #95 credential-recovery repair, retaining
its Linux error categories and unsaved input restoration.

Only A (syllable) and B (ribbon) remain. B is the default; legacy classic and
unknown saved values migrate to B without changing the animation preference.
Compact indicators grow to 24px. The empty overlay has a prominent 48px/80px
variant scaled to the space remaining after the control band and status line.
Stronger strokes and less compounded opacity improve visibility while
preserving explicit animation-off and reduced-motion behavior.

The expanded panel uses two compact selectors, a single-line audio status and
single-line immersive/lock switches. Its native width is 280px and initial
height 270px; React still measures the actual localized height. On macOS the
status reports the current default system output name, read without changing
ScreenCaptureKit's mixed system-audio capture route. A missing name omits the
unknown-device placeholder. Device names stay in the local UI and never enter
logs or copied diagnostics. Windows retains its audio-output settings entry.

Turbo is the sole active translation mode. Old mode values remain readable
and normalize to Turbo through configuration validation, preference snapshots,
saves and language changes; no mode selector remains. ASR source drafts appear
as soon as a free preview lane permits them. A successful nonempty HQ preview publishes one atomic source/translation
pair, bounded to one pair and 64 KiB per lane. While later ASR or HTTP work
progresses, the last complete pair remains visible; short SSE prefixes and empty
drafts cannot erase it. Preview pairs never enter saved history. Valid confirmed
pairs replace previews atomically; invalid empty finals preserve the last pair.
Preview HTTP pending has its own request owner and does not borrow final pending
semantics. Cancellation, stale owners and lifecycle boundaries clear that signal.
Confirmed source/translation pairs remain atomic and durable.
Replaceable previews use 250ms stability and a 1s maximum wait. Meaningful short
unpunctuated drafts can preview without the old 12-character gate. Final work
keeps priority, and cancellation, bounded queues and stale-worker guards remain.
Actual MT HTTP starts now share a 1.1s minimum floor and a conservative local
450-unit/s input/output work estimate, including bounded translation memory.
This is an engineering budget, not measured provider token usage. Previews use
the newest two confirmed memory pairs; final requests retain six. Preview starts
have a 1.5s base interval, increasing with cumulative input size; punctuation or
spacing-only changes do not resend, and corrections without meaningful growth
refresh at most once per five seconds. Waiting work retains only the latest
candidate. Final work bypasses this preview cadence, but obeys the common HTTP
floor, charged input budget and service backoff. A 429 stops preview retries and
pauses previews for 30s, reserving recovery requests for ordered finals. A final
success cannot refund that pause. Pause/reconnect transfers only timing/budget
metadata within the same profile, provider, translator route and model; it never
transfers draft fingerprints or translation memory. Explicit stop/new-session
resets this local policy, without promising that the server quota has recovered.
No draft timer creates an authoritative final or rewords the provider prompt.
This improves local waiting and presentation; music recognition quality still
requires actual provider/session observation.
Explicit ASR-reported same-language output bypasses text translation. The
whitelist is English/English, Japanese/Japanese, and clearly simplified or
generic Chinese/Simplified Chinese; unknown and traditional Chinese reports
retain MT. Confirmed passthrough remains in the serial final queue so an
earlier cross-language request cannot be overtaken. Passthrough clears the
latest MT timing, does not invent a zero-duration request, and does not add
translation memory. Epoch cancellation and deferred preview guards still apply.

The normal overlay reserves a 61px active/error top band, separating the drag handle
from the native capsule and leaving room for timing labels. Short lines use
actual measured height instead of occupying their full line budget. Following
live output bounds both confirmed and live presentation to finite tails; a new
sentence no longer expands its predecessor into a wall. Upward wheel,
Home/ArrowUp/PageUp or a deliberate touch scroll opens complete confirmed
history and the current replaceable live text with the same reading anchor.
Reading a live row does not confirm it or retain its earlier revisions. End,
the visible return-to-live action, or returning to the bottom resumes
compact following. A ResizeObserver gives each bilingual lane the number of whole lines that
fit the actual body height, including 1+1 in short windows. Absent source text
leaves the available space to translation.
Each following row uses that viewport-sized bounded lane budget. Starting the
next short phrase no longer overrides the preceding confirmed row to one line
per language: a three-line English phrase could otherwise turn into only its
last word despite spare body space. The complete text was still in the DOM;
this was a presentation cut, not stabilization or history truncation. The
outer timeline continues to scroll to the latest tail, so older rows can leave
the viewport naturally. Long preceding paragraphs still have finite lane
heights, and only explicit reading opens their full text. Three-mode component
fixtures reproduce the former three-lines-to-one transition with measured line
heights, then verify that fitting lines survive the next phrase without
remounting; short-window bounds and reading/End behavior remain covered.
The native minimum expanded height is 136px so the controls and default-sized
bilingual lanes fit; the collapsed strip keeps its existing smaller height.
Complete text stays available in the DOM/accessibility tree and local history
retention remains independently opt-in.
Subtitle bodies render the provider text as plain text without inline loading
dots or a cursor. Waiting and translation status stay in the fixed timing and
capsule surfaces. Genuine punctuation remains; no clipped-history continuation glyph is added.
A streaming-state change alone cannot add glyphs or alter measured lane height.
Sentence dividers default off and remain a manually enabled subtitle setting.
When enabled, ordinary overlays show a full inset-width neutral line between sentences;
immersive overlays retain space only. Toggling preserves history-reading state
and the reading anchor, and never changes recognition or translation settings.
Short subtitle output aligns with the lower reading edge; overflowing history
keeps its ordinary scroll flow. Error states retain the same capsule clearance
and right-side controls, with the first action restarting the session.
Narrow windows retain that primary pause/retry action even when the complete
secondary action row does not fit.

The native control capsule measures its intrinsic content width separately from
the 280px expanded panel. Its left anchor remains fixed while language, locale
and transient status change. A hidden measurement-only capsule keeps that width
current even while the panel is open; delayed width reports only resize island
mode and cannot reveal a hidden window or shrink the panel. Existing work-area
clamps and upward expansion on short screens remain unchanged.

Overlay icon actions show visible labels on hover and keyboard focus through a
shared body-portal tooltip. Positioning clamps to the native WebView bounds;
the 54px collapsed strip uses side placement when vertical space is insufficient.
Busy controls show a bounded spinner, expose their updated label and reject
duplicate clicks. Tooltip labels remain localized by the existing action copy.
On macOS the subtitle panel stays nonactivating. Its WKWebView receives hover
through an independent public AppKit ActiveAlways tracking owner, without
changing Tao's delegate or monitoring global events. View-local logical points
are converted to CSS coordinates once and emitted only to the overlay. Movement
is coalesced at 16ms with one latest pending point and a one-shot flush; leaving,
locking, hiding, destruction and exit clear the target. The renderer subscribes
once and updates only a changed hit target. Clicking dismisses the tooltip;
keyboard focus-visible can show it, but click focus cannot retain it.
Mouse movement can open the hint without a preceding enter event. Clicks and
Escape dismiss it until the pointer leaves, and pointer focus never keeps a
hint open. Keyboard focus-visible retains the same readable label. A single
window-scoped native movement bridge supplies hit targets for macOS's
nonactivating overlay; it never activates or focuses the window. Only target
changes update control hover feedback, so repeated movement over one action
does not rerender the subtitle window.

Normal app exit remains in the tray footer and is also available in a fixed
settings-sidebar footer, outside its independently scrolling navigation/help.
Both entries invoke the existing stop/finalize/exit workflow without a second
confirmation, show pending feedback, guard duplicate requests and allow retry
after a brief localized failure. Settings receives the existing app_quit grant;
the subtitle overlay does not gain an exit control.

The native tray menu and macOS application-menu Quit/Cmd+Q use the same
stop/finalize/exit function. A failed archive finalization keeps the app open and
opens the existing export page's safe save-error feedback and retry action.
The macOS application menu replaces only its predefined Quit with a localized
ordinary menu item; About, Services, Hide and the complete Edit/Window menus
remain intact. Ordinary Dock Quit is intercepted through the public AppKit
termination delegate and shares this boundary too; see
`2026-10-02-native-quit-finalization.md`. `ExitRequested` alone does not cover
AppKit's direct termination in the locked runtime. OS shutdown and Force Quit
are not claimed as safe-finalization paths.

Getting started is a persistent, manually opened settings-sidebar entry with
three steps and service/subtitle navigation. It does not start capture, save
credentials/preferences or write a completion flag. Redundant sidebar borders
and the details disclosure are removed; sanitized diagnostic copy, GitHub
feedback and a visible manual-copy fallback remain.

Streaming revisions replace a whole settled phrase without per-word opacity
animations. A sole recognized lane uses the primary font size and contrast. The bilingual
reference lane keeps the same smaller size before and after translation arrives,
so its arrival cannot shrink and rewrap the original. New blocks move upward
2px over 140ms without changing letter opacity. Roll-up motion is position-only
and respects the animation and reduced-motion preferences.
Confirmed long utterances remain bounded while following and expand on explicit
history-reading intent. Optional sentence dividers default off and, when enabled,
use a full inset-width neutral line; immersive mode keeps spacing only.

The normal expanded overlay shows the latest API round trip from the existing
connection health check, without additional requests or polling. Translation
duration covers a successful nonempty current preview or final text request,
including retries but excluding ASR and queue work. The legacy
end-to-end client measures matching source-final to translation-final receipt
as translation wait; a translation received first has zero additional wait.
Active work displays a pending or translating label rather than an unexplained
dash; MT recovery displays a short explicit rate-limit/retry status. Unsupported
or inactive observations show an em dash. Timings are
content-free and clear across inactive/paused/error/reconnecting or stale
generations; original-only mode has no translation sample. Immersive mode
hides the API and translation timing row. It retains explicit control-action
feedback and the return-to-live reading action without introducing another
panel or shifting the subtitle content origin.
Only completed numeric samples receive a presentation color: API measurements
are neutral below 500ms, amber from 500ms, and pale red from 1500ms; translation
request/follow measurements are neutral below 1000ms, amber from 1000ms, and
pale red from 3000ms. These are UI experience bands, not network-only RTT,
end-to-end subtitle latency, provider SLAs, or failure classifications. Pending,
recovery and inactive labels stay neutral rather than inheriting stale samples.
Characters per second is omitted:
provider batching and sentence length make it less useful than elapsed time
for diagnosing the user's reported delays.

Verification covers timing/pairing/lifecycle and migration tests, short-preview
HTTP fixtures, bounded layout and reading behavior, settings navigation and
clipboard fallback, all three UI languages, motion preferences, the canonical
repository check and signed native inspection. UI fixtures and CI establish
correctness, not real service latency or music recognition quality. Real
provider timing requires a user-started session and is reported separately.

Settings groups subtitle appearance/reading, service and recognition/translation
languages, recording/export, application preferences, and diagnostics. Session
controls appear only in subtitle and service pages. A separate diagnostics page
prepares a content-free snapshot on entry, offers a manual refresh, copying and
the existing fixed GitHub feedback action. Sidebar help remains a guide entry
and one aligned normal-exit action. Hidden service editing state is retained to
preserve unsaved write-only credential drafts across category navigation.

Transient MT failures share one cooldown across replaceable preview tasks.
Rate-limit retries preserve capture and recognition during the bounded cooldown;
final work remains serial with the existing bounded queue and deadline. Exhausted
transient final work uses visible bounded reconnection, while authentication failures
remain explicit terminal errors. Manual stop, pause, reconnect and stale-generation
guards continue to cancel their owned work. Overlay start is explicitly authorized
in the window-scoped IPC permission and acknowledges clicks while its Promise is
pending, rejects duplicate clicks and surfaces IPC failures.
An exhausted replaceable preview records that no retry remains scheduled; a quiet
input cannot keep claiming an active retry. New admitted work, success and lifecycle
boundaries refresh or clear this state. Paused settings expose a dedicated resume
action with bounded pending feedback and their own window-scoped IPC grant.

Development installation retains one canonical signed application. After a
successful install, the disposable build app bundle is removed while incremental
Cargo artifacts remain reusable. Formal release installation is untouched.

Service configuration uses the existing unified selectors throughout. The
Alibaba text override has three routes: follow the current service, DeepL's
official API with its preset host, or a separately configured custom service
using the existing DeepLX JSON contract. DeepL and custom secrets occupy one
profile-scoped Keychain record with distinct fields; switching routes preserves
both and never moves either key into the other route. Existing legacy custom
profiles retain their original ASR credential item and migrate destinations
without changing its value. Failed reads can be retried after unlocking.
DeepL does not add a capture provider: Alibaba continues recognizing system
audio. The official HTTP client bounds response sizes, disables redirects,
supports cancellation and reports only sanitized error categories.

Checking a connection waits for the provider's authenticated ready event and,
for split recognition/translation routes, validates a fixed synthetic text
request. Transport reachability alone cannot produce "Available". Probes send
no system PCM or user subtitles, close their connections on every result, and
return a concise status with a short safe reason. The provider list uses
bundled official logos with uniform sizing and recorded asset provenance.

The service page keeps session status, its switch and an explicit error retry in
a compact row. Translation backoff reports its short recovery status while
recognition continues. Connection checks show "Connection available" beside
their button; an active session error clears that profile's earlier check and
invalidates in-flight results. Credential actions stay together beneath the
check, with only the short secure-storage note. Overlay retry has explicit
`session_start` authority; content-free entry and skipped labels distinguish IPC
delivery from lifecycle guard decisions.

The dedicated diagnostics page copies a versioned safe snapshot with the
latest 24 typed lifecycle, status, failure, recovery and translation-backoff
events. Times are monotonic milliseconds since app start, accompanied by an
event sequence and omitted-event count. Source/translation draft and final
events increment saturating counters without retaining text or displacing
state transitions. Accepted generation checks reject stale provider events;
the existing bounded stopping tail may still count confirmed final pairs.
Current latency and translation-recovery fields use the same validity rules
as the overlay. Fixed MT rate-limit, temporary-service and backlog labels stay
distinct. The journal is memory-only and independent of recording/history
preferences, has no raw-log input, and excludes keys, text, endpoints, paths
and device/profile names. Reports remain valid JSON within 16 KiB; oversized
GitHub links use the existing fixed-destination manual-paste fallback.

Frontend session snapshots reuse the bounded confirmed-history array only when
every item's timestamp, source and translation match. Drafts and lifecycle
fields remain fresh. Timeline blocks depend on settled text and primitive
streaming state, so raw draft churn cannot bypass display stabilization. Earlier
history corrections, appended pairs, clearing and trimming still update at once.

While following, a preceding bilingual sentence keeps one visible line in each
selected language; scrolling upward opens its full text. The newest
sentence uses the actual measured viewport height, rather than a fixed two-line
limit. Long bilingual lanes start with 50% of available height each, with spare
height from either measured short lane given to the longer one. At 143px of
available height and subtitle size 17, this changes the long/long presentation
from two original plus four translated lines to three plus three. The native
minimum still keeps one line in each language. Independent wrapping and tail
clipping do not provide sentence alignment; complete pairs and full reading
remain intact. A sole language uses all the
available height. Reference text is 90% of the primary size at every window height,
as updated by the [readability design](2026-10-20-subtitle-readability-and-drag.md),
and 86% white; integer line heights keep glyphs inside the clipping boundary.
Upward reading intent restores full confirmed and current live text. No edge fade or continuation
glyph is inserted into the text. The optional divider is one full CSS pixel, at
34% white, across the inset body width, with 7px breathing room. It remains
default-off and persists.

Service rows share one hover surface and do not repeat an unchanged provider name.
The detail view aligns language support, profile name and connection/credential
actions to the title's content column. Name saving appears only after an edit.
The diagnostics page shows a compact status summary; events and raw safe JSON
are mounted only inside the closed-by-default details disclosure. Opening or
explicitly refreshing the page reads one bounded snapshot, with no polling.

Bootstrap settings and session streams progress independently. Register each
listener before its snapshot, prefer buffered new events, and give each native
step a 12-second UI deadline. Expired attempts release every existing or late
subscription and never publish late replies. Show loading or an explicit manual
reload action rather than treating initial placeholder credentials as a failed
Keychain read. The whole explicit connection check has a 30-second UI deadline;
timeout means the check did not finish, not that the service rejected access.
No repeated automatic OS-credential reads are scheduled. These UI deadlines do
not cancel an already running native OS operation.

Saved credentials remain absent from ordinary snapshots and persistence drafts.
An explicit settings-only request may reveal one profile-scoped field temporarily.
Hiding, cancelling, navigating, route/profile changes, document hiding, blur and
native close-request events clear that value and expire pending replies. Observe
the native close event without invoking the window API's automatic destroy path.

Native playback acceptance uses downloadable licensed reference media across
slow/normal/fast speech and singing. Compare audio receipt, first source draft,
translation completion, stale/error recovery, narrow/short overlays, and history
reading. Keep measurements content-free and document material limits separately
from automated fixture results.

The nonactivating macOS overlay uses its existing local tracking owner for
button cursors as well as hover. AppKit does not deliver `cursorUpdate` to an
`ActiveAlways` tracking area, so an overlay-only `{x, y, pointing}` command
applies the public `NSCursor` hand/arrow on the main thread and returns whether
the intent was accepted. Only the latest delivered point on a visible,
non-passthrough overlay may accept it. The renderer bounds requests to one in
flight and retries a rejected stale point only after a new actual movement.
Real native motion may reapply this owner's hand. Leaving, hiding, locking,
or removing tracking restores the arrow only if this owner set the hand;
there is no activation, global monitor, extra listener, or polling timer.

Preview tasks retain a fixed-size candidate identity for their lifetime.
Duplicate/cosmetic updates preserve an identical waiting or in-flight task;
reverting A → B waiting → A cancels B before the completed-A deduplication
returns, rejects late B partials, and restores the latest source display.
Actual request accounting and final ordering are unchanged.

Same-language and Original-target sessions select current recognition as the
reading text even in translation-only display. A stale mirrored translation
cannot override it or reappear after the original was confirmed into history.
Pair-only preview content also exposes the clear action. Real OverlayWindow and
Timeline DOM regressions cover mode changes, clear, pause/resume and reconnect;
they do not substitute for native viewport inspection.

The global Direct / System / Custom network preference routes both recognition
WebSockets and translation HTTP through the same immutable session snapshot.
Changes require a fully stopped session and invalidate old probe results without
discarding unsaved profile fields. Supported protocols and deliberate exclusions
are recorded in the separate provider-network-proxy design. Native system text
translation is deferred at the user's request; this iteration retains third-party
services and does not advertise a local translator.
