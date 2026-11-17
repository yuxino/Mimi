# Floating subtitle controls and service details

The expanded overlay uses a single control row. The language capsule opens the
control panel, and the action strip offers pause or resume, collapse and close.
Keep a 224 px maximum for the capsule, including on an embedded surface. When
space is limited, prioritize close, then the session action, then collapse.
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
Devices without fine hover keep the controls visible. Hidden controls stay
reachable by keyboard and release their pointer targets. Live subtitles
reclaim the control row when quiet. Pending or failed actions, translation
recovery and Back to live can still occupy one short status line. The listening
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
- Put audio input second. Keep the existing system sound, microphone and
  application choices according to platform availability. Keep status and
  device details in the existing help controls.
- Put subtitles third. Offer the original, translated and bilingual display
  preferences as three directly selectable buttons. The Subtitles page in
  Settings holds the draft display, sentence dividers and timestamps options.
- Place immersive mode and position lock side by side above the bottom row.
  Put Clear subtitles and More settings in that row. Clear calls the existing
  subtitle action and is available when the timeline has content. Serialize clearing
  with the panel's other settings actions and show a sanitized failure if
  clearing fails.

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
`scripts/verify-overlay-service-layout.js` for synthetic browser cases. Check the
panel and caption surface at narrow widths, both system color schemes and with
long localized labels. Include built-in and independent routes, loading and
recoverable error states, plus bilingual and timestamped captions at minimum
heights. The browser harness imports the actual loaded module URLs, including
Vite's version queries, and verifies that the mounted UI received its
synthetic state. Refuse native-bridge pages and keep these fixtures in memory.

In the signed native preview, check control entry and departure across the
two surfaces, keyboard focus, panel scrolling and the resting caption layout.
Record any native verification and its limits in the integration run ledger.
