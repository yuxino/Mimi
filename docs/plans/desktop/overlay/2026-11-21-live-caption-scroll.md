# Keep the reading edge stable during live translation

In [#397](https://github.com/yuxino/Mimi/issues/397), the same lines of a bilingual caption moved down and then up as the next sentence arrived. Two geometry changes can disturb the timeline while it follows the newest text.

The expanded canvas gives live translation a 20-pixel status line once captions are present. Pending, preview, retry and completion share that line. Moving the pointer away still returns the toolbar's height to the reader. Recognition-only sessions use the space when an action or message needs it. Immersive captions keep their existing layout.

On its first layout a compact lane takes the height of its text up to its line limit. When a paragraph already exceeds the limit, the newest lines meet the bottom of the lane on that first layout. This applies to live lines and to a newly mounted history row after confirmation.

Track a surviving historical line across a pending request, its cancellation, translation arrival and the next sentence. With the pointer fixed, the pending label alone must keep both viewport edges and that line in place. As new lines arrive, the history may move up. Test with motion both on and off. Keep the normal reading gesture and explicit return-to-live behavior.
