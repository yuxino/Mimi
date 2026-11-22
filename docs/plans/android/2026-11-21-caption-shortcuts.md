# Start and stop Android captions from another app

People watching a video can add the Mimi captions tile to Android Quick Settings. Two independent preferences in More also offer a floating edge button and an idle notification. Both preferences start off. The tile uses the selected inputs and service from Mimi. When capture stops, every entry reflects the same service state. These controls do not switch inputs or enable recording.

## Starting and returning

All three start actions enter a private translucent activity in a separate task. It shares the main window's capture-start flow and shows Android's permission dialogs over the app being watched. Each system-audio session needs fresh sharing consent. An external start is consumed once, including across rotation or a second tap while a dialog is open. Missing configuration opens the service settings. After cancellation or service acceptance the shortcut task closes, returning to the previous app.

## Edge and notification

The floating shortcut shows Mimi peeking in from the left or right side of the display. The portrait has a transparent background and a 30 dp visual height inside a 48 dp touch target. The artwork faces into the screen on either side. A drag sets the side and height, and the saved vertical position is relative to the usable display area. Dragging moves the shortcut without operating captions.

The first tap reveals one button beside the portrait, toward the center of the display. It pairs a play triangle with **Start** or a square with **Stop**. Screen readers announce the full action, **Start captions** or **Stop captions**. The portrait stays in place. For touch input, the separate button performs the action. Tapping the portrait again leaves capture unchanged. After three idle seconds the button retracts and the portrait remains. With touch exploration enabled, the command stays available. An accessibility service can activate it on the focused portrait or move the shortcut to the opposite side. Mimi hides the shortcut while its own UI is in front or the phone is locked. Opening Mimi restores the enabled shortcut after Android reclaims its idle process.

On Android 13 and later, the main screen may offer notification permission once after dispatching capture on a fresh installation. It records that offer before opening the dialog, including when the user dismisses it. An upgrade preserves a previous installation’s notification choice. A shortcut goes straight to the required capture permissions. Using the idle notification switch in More also settles the automatic offer. [Android notification permission](https://developer.android.com/develop/ui/compose/notifications/notification-permission)

The optional idle notification contains a Start action. During capture the service already owns an ongoing notification with Stop. Once the service releases its resources, the idle notification can return. Android controls channel availability, lock screen presentation and user dismissal. Turning off the idle notification preference never stops an active session.

Locking the device ends capture. When an active system-audio-only session ends because of the lock, the process remembers that interruption until unlock. If notifications are allowed, it offers one dismissible Continue action using the same notification slot. This works with the persistent idle notice disabled. Continuing requires fresh Android consent and a matching system-only input selection. An explicit Stop, a permission failure, or a session using the microphone never creates this reminder.

New system-audio sessions, including unlock continuations, use Android’s standard sharing consent. Shizuku integration remains a separate request in [#401](https://github.com/yuxino/Mimi/issues/401).

The tile, edge button and notifications belong to the Android adapter. Gemini connection rotation remains in the shared provider runtime. [Android Quick Settings](https://developer.android.com/develop/ui/views/quicksettings-tiles) · [Screen-sharing consent](https://developer.android.com/media/grow/media-projection#user_consent)

## Main-window hierarchy

More starts with the shortcut group: Add to Quick Settings is the first action, followed by the independent edge and idle-notification switches. Interface language is separate from Help and about. The guide, content-free diagnostics and installed version/update check share that lower group. The version and check action use one row when their measured labels fit and stack at narrow widths or larger text sizes.

The caption page keeps the selected service beside its input/language choices, before display, appearance and storage controls. More omits the idle start/setup footer; an unconfigured service page omits its duplicate setup action. Captions always retains the primary action, configured services can start directly, and every page retains session controls while starting, running or stopping. A settings-load failure exposes its existing recovery state. Page switching preserves scroll position and keyboard focus; it never starts capture or changes a shortcut preference.
