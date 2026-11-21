# Keep the reading edge stable during live translation

In [#397](https://github.com/yuxino/Mimi/issues/397), the same lines of a bilingual caption moved down and then up as the next sentence arrived. Two geometry changes can disturb the timeline while it follows the newest text.

When the reader hovers over the expanded canvas or reaches the controls with the keyboard, a 20-pixel line below the toolbar can show translation progress. Its height stays fixed across pending, preview and completion. Moving the pointer away hides ordinary progress and gives both the toolbar and status space back to the captions. Rate limits, service failures and explicit action feedback stay readable. Touch devices retain their visible controls.

On its first layout a compact lane takes the height of its text up to its line limit. When a paragraph already exceeds the limit, the newest lines meet the bottom of the lane on that first layout. This applies to live lines and to a newly mounted history row after confirmation.

Track a surviving historical line across a pending request, its cancellation, translation arrival and the next sentence. With the pointer fixed, the pending label alone must keep both viewport edges and that line in place. As new lines arrive, history may move up. Check the painted label with the pointer both inside and outside, and with motion on and off. While reading history, move directly from outside the overlay to the visible return button and click its original coordinates. The first click must resume live following.
