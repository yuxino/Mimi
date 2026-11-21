# Keep the reading edge stable during live translation

In [#397](https://github.com/yuxino/Mimi/issues/397), the same lines of a bilingual caption moved down and then up as the next sentence arrived. Two geometry changes can disturb the timeline while it follows the newest text.

On hover or keyboard focus, the language button stays on one line. A separate line of plain text beside the tools pairs translation activity with one measured time. Prefer the latest API round trip, or use the measured translation time when the service has no round trip. The panel labels all available readings. While open, its header owns translation progress. Narrow windows place the readout below the tools and reserve that space while hovered so the first sample cannot push a sentence down. Moving the pointer away gives the chrome space back to captions. Rate limits, service failures and explicit action feedback stay readable. Touch devices retain their visible controls.

The language button and the toolbar are part of the same hover interaction even when they occupy two native windows. Entering the caption text reveals both. Moving onto the language button keeps it visible and the first click opens the two language selectors. Verify the pixels of the child window separately from the canvas. A visible native window or an accessibility label can still have a fully transparent button that ignores clicks.

AppKit can show the language window with its parent before Mimi explicitly orders it front. Install its mouse tracking area in either case. After scrolling back and clicking “Back to live”, move straight from outside onto the quiet language button. Both controls must paint before the first press, and that press must open the panel.

When that panel closes, the caption window can regain focus while the pointer is outside both windows. WebKit may deliver a new pointer-enter at that moment. Once AppKit has reported the physical pointer for a window, use its tracking area for subsequent entries and exits. A real keyboard focus on the controls continues to reveal them. Check a second open and close after reading history, then enter again with the mouse.

On its first layout a compact lane takes the height of its text up to its line limit. When a paragraph already exceeds the limit, the newest lines meet the bottom of the lane on that first layout. This applies to live lines and to a newly mounted history row after confirmation.

Track a surviving historical line across a pending request, its cancellation, translation arrival and the next sentence. With the pointer fixed, the pending label alone must keep both viewport edges and that line in place. As new lines arrive, history may move up. Check the painted label with the pointer both inside and outside, and with motion on and off. While reading history, move directly from outside the overlay to the visible return button and click its original coordinates. The first click must resume live following.
