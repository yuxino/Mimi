# Reuse unchanged subtitle lanes

The desktop overlay retains at most 20 confirmed rows. A new displayed live
chunk rebuilds projected block objects, including those retained rows; memoizing
the complete Timeline does not skip the unchanged lanes inside that update.

Keep the existing rows and layout algorithm. Memoize Lane using its actual
presentation props, share the resolved font family for the selected font, and
give only the final row stable measurement callbacks. A row becoming historical
must receive an absent callback so a late resize cannot replace the new final
row's measurements. Font, size, alignment, motion and compact/reading changes
remain ordinary props that invalidate the affected lanes. Locale subscriptions
stay on Timeline for source metadata; the caption fallback remains the shared
language-aware CSS font variable.

A temporary probe against the actual components used 20 retained bilingual rows
and 20 displayed live updates. Lane and CompactLane calls each fell from 840 to
40; unchanged historical Lane calls fell from 800 to zero. Measurement-ref
layout effects fell from 840 to 40, and repeated font-family normalization fell
from 840 to zero after mounting. No geometry reads occurred in either measured
loop. Timestamp formatting still ran 400 times in both versions and is outside
this change.

React/jsdom reported about 118 ms versus 78 ms of render work in those runs.
These timings describe the local synthetic probe; they do not establish native
frame rate, provider latency or a production CPU reduction. Focused regressions
cover freshly allocated history objects, immediate displayed text, final-row
measurement ownership, reading identity, font/size edits and all eight locales.
Browser geometry/motion and complete acceptance remain separate checks.
