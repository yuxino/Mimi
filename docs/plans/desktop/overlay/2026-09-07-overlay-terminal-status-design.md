# Overlay terminal status

Native acceptance exposed a mismatch after ScreenCaptureKit denied capture:
Settings correctly showed a stopped session with a permission error, while
the separate overlay control still announced Listening. The shared activity
model mapped both error and idle to listening.

Give idle and error their own activity phases. Terminal status takes priority
over stale pause, recognition, and translation flags. Both phases use a static
indicator; the control capsule shows a short localized status as well as its
accessible label. Existing detailed errors stay in Settings and the normal
subtitle canvas. Immersive subtitle presentation and permission handling do
not change.

Regression coverage exercises terminal states with all stale activity flags
set. Verify the localized control and stopped indicator in the isolated signed
development app, and keep public-package capture acceptance separate.
