# Start and stop Android captions from another app

People watching a video can add the Mimi captions tile to Android Quick Settings. Two independent preferences in More also offer a floating edge button and an idle notification. Both preferences start off. The tile uses the selected inputs and service from Mimi. When capture stops, every entry reflects the same service state. These controls do not switch inputs or enable recording.

## Starting and returning

All three start actions enter a private translucent activity in a separate task. It shares the main window's capture-start flow and shows Android's permission dialogs over the app being watched. Each system-audio session needs fresh sharing consent. An external start is consumed once, including across rotation or a second tap while a dialog is open. Missing configuration opens the service settings. After cancellation or service acceptance the shortcut task closes, returning to the previous app.

## Edge and notification

The optional edge control keeps a 48 dp touch target and reserves that area for dragging on gesture-navigation devices. After three idle seconds it moves to the nearest edge, respecting the system animation setting. A deliberate drag saves its position along that edge as a fraction of usable space. The first tap reveals the action and the next tap operates captions. Touch exploration keeps it expanded and offers an action to move it. It disappears while Mimi is in front or the device is locked. Opening Mimi restores the button and its position after Android reclaims the idle process.

The complete color app icon stays at 34 dp and 80% opacity while tucked, pressed or expanded. Its position follows the drag. Touching the control reveals a separate start or stop badge, which disappears when the control tucks. The logo stays in place during both transitions, and playback remains visible through its transparent corners. The small Android header and edge control use the existing 64 px and 128 px renditions so the fine lines stay smooth. Android's monochrome Quick Settings tile and notifications use the desktop five-bar tray template. `npm run icons:generate` copies these assets from the desktop set and `npm run icons:verify` checks their bytes.

The optional idle notification contains a Start action. During capture the service already owns an ongoing notification with Stop. Once the service releases its resources, the idle notification can return. Android controls channel availability, lock screen presentation and user dismissal. Turning off the idle notification preference never stops an active session.

Locking the device ends capture. When an active system-audio-only session ends because of the lock, the process remembers that interruption until unlock. If notifications are allowed, it offers one dismissible Continue action using the same notification slot. This works with the persistent idle notice disabled. Continuing requires fresh Android consent and a matching system-only input selection. An explicit Stop, a permission failure, or a session using the microphone never creates this reminder.

The tile, edge button and notifications belong to the Android adapter. Gemini connection rotation remains in the shared provider runtime. [Android Quick Settings](https://developer.android.com/develop/ui/views/quicksettings-tiles) · [Screen-sharing consent](https://developer.android.com/media/grow/media-projection#user_consent)

## Main-window hierarchy

More starts with the shortcut group: Add to Quick Settings is the first action, followed by the independent edge and idle-notification switches. Interface language is separate from Help and about. The guide, content-free diagnostics and installed version/update check share that lower group. The version and check action use one row when their measured labels fit and stack at narrow widths or larger text sizes.

The caption page keeps the selected service beside its input/language choices, before display, appearance and storage controls. More omits the idle start/setup footer; an unconfigured service page omits its duplicate setup action. Captions always retains the primary action, configured services can start directly, and every page retains session controls while starting, running or stopping. A settings-load failure exposes its existing recovery state. Page switching preserves scroll position and keyboard focus; it never starts capture or changes a shortcut preference.
