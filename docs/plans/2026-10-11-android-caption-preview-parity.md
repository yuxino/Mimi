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
slider being dragged. Normal mode shows complete bounded content inside the
same scrolling panel; immersion retains the real compact caption line limits.

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
