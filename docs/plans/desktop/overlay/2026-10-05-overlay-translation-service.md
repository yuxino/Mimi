# Floating subtitle controls and service details

The expanded overlay uses a single control row. The language capsule opens the
control panel, and the action strip offers pause or resume, collapse, clear,
immersive mode, position lock and close. Keep a 156 px maximum for the capsule,
including on an embedded surface. Preserve the full accessible language label
when its visible text truncates. At the supported 360 px minimum width all six
actions retain 24 px targets and remain separate from the capsule.
The empty drag area moves the window. Collapse and expand through their
explicit buttons.

Use the existing dark overlay, typography and provider marks. The initial
expanded frame is 580 × 240 logical pixels. Honor saved frames and the native
and browser minimum sizes for the chosen subtitle layout, including the room
that bilingual text, timestamps and two audio inputs require. Keep the
caption timeline mounted and preserve the bottom reading edge as the controls
appear, disappear or switch between modes.

On the regular expanded canvas, hover and keyboard focus reveal the capsule
and action strip. Errors and an open control panel keep controls available.
Keep the capsule visible during recording and while the overlay is locked.
The locked capsule exposes one-click unlock while the subtitle canvas remains
click-through. An error temporarily restores the regular recovery controls.
Devices without fine hover keep the controls visible. Hidden controls stay
reachable by keyboard and release their pointer targets. Live subtitles
reclaim the control and progress rows when quiet. Session actions, failures and
translation recovery retain a short status line. While reading history, Back to
live stays below the captions, within reach as the controls appear. The listening
indicator and its text retain their place before captions arrive.

Detached surfaces exchange exposure updates through targeted Tauri events
and retain their own hit testing and geometry. DOM hover and macOS pointer
evidence work independently. Focus loss does not count as pointer departure.
A brief hold spans movement between the two neighboring surfaces. On native
Wayland the embedded panel shares the subtitle surface and compositor movement.

## Expanded control panel

Use a 280 px panel with a maximum height of 520 px, limited further by
available native or embedded space. Allow scrolling to every control at
constrained heights and with long localized labels. Keep headings, spacing
and control size consistent at this width.

- Put speech translation first. Source and target use two equal-width columns,
  with the label and its optional help beside each other above the picker.
  Show the current configuration in a full-width picker. Choosing Original
  in the target picker uses recognition alone. Select a target language to
  start translating again.
  Omit a repeated provider-brand row for a single built-in service; preserve
  distinct recognition/translation routes and custom recognition names.
  Timing and recovery observations remain available independently.
- Put audio input second. Keep the existing system sound, microphone and
  application choices according to platform availability. Keep status and
  device details in the heading help. Ordinary source buttons show hover
  feedback without opening a large tooltip over neighboring controls; their
  detailed descriptions remain available to assistive technology.
- Put subtitles third. Offer the original, translated and bilingual display
  preferences as three directly selectable buttons. The Subtitles page in
  Settings holds the draft display, sentence dividers and timestamps options.
- End the panel with a compact, right-aligned More settings action. Keep
  immersive mode and position lock in the toolbar rather than duplicate them
  in this configuration panel. Clear subtitles belongs beside Pause,
  Collapse and Close in the expanded overlay toolbar. Disable Clear while empty
  or another session/control action is pending; preserve the existing guarded
  action and sanitized retryable failure feedback. Keep the capsule and toolbar
  separated at the minimum supported width.

Keep visible labels and values readable as they wrap. Help remains independently
focusable. Give open pickers an opaque surface so their options are readable
over the controls behind them.

## Recognition and translation observations

Show the actual service route and measured timings below the panel's current
configuration. The details subscribe to the settings and session observations
they render. The subtitle canvas shows the status needed to keep reading,
including translation recovery and Back to live. Hover or focus on the service
reveals the full route, and clicking opens the active configuration in Settings
through the panel's guarded action.

Use credential-free settings metadata. Independent services appear in
recognition-to-translation order. A built-in translation route following its
recognition service appears once. A legacy DeepLX configuration pairs Alibaba
speech recognition with DeepLX translation. An Original target or a custom
recognizer without a text route shows its recognition service. The subtitle
display preference controls the visible lanes independently of the live
translation target. A configuration alias can describe a custom stage. Use
the provider and route metadata to identify it. Bound long inline names and
keep the full names available in the tooltip.

## Verification

Use focused route and action tests, the repository check and
`scripts/frontend/verify-overlay-service-layout.js` for synthetic browser cases. Check the
panel and caption surface at narrow widths, both system color schemes and with
long localized labels. Include built-in and independent routes, loading and
recoverable error states, plus bilingual and timestamped captions at minimum
heights. The browser harness imports the actual loaded module URLs, including
Vite's version queries, and verifies that the mounted UI received its
synthetic state. Refuse native-bridge pages and keep these fixtures in memory.

In the signed native preview, check control entry and departure across the
two surfaces, keyboard focus, panel scrolling and the resting caption layout.
Record any native verification and its limits in the integration run ledger.
