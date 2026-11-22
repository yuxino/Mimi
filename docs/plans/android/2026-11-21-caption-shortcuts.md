# Start and stop Android captions from another app

People watching a video can add the Mimi captions tile to Android Quick Settings. Two independent preferences in More also offer a floating edge button and an idle notification. Both preferences start off. The tile uses the selected inputs and service from Mimi. When capture stops, every entry reflects the same service state. These controls do not switch inputs or enable recording.

## Starting and returning

All three start actions enter a private translucent activity in a separate task. It shares the main window's capture-start flow and shows Android's permission dialogs over the app being watched. Each system-audio session needs fresh sharing consent. An external start is consumed once, including across rotation or a second tap while a dialog is open. Missing configuration opens the service settings. After cancellation or service acceptance the shortcut task closes, returning to the previous app.

## Edge and notification

The optional edge control keeps a 48 dp touch target and reserves that area for dragging on gesture-navigation devices. After three idle seconds it moves to the nearest edge, respecting the system animation setting. A deliberate drag saves its position along that edge as a fraction of usable space. The first tap reveals the action and the next tap operates captions. Touch exploration keeps it expanded and offers an action to move it. It disappears while Mimi is in front or the device is locked. Opening Mimi restores the button and its position after Android reclaims the idle process.

The 48 dp target carries one small charcoal tab. Its outer end meets the display edge and its inner corners are rounded. The tab sits centered under the finger in free travel. Only the last 16 dp near an edge progressively changes its silhouette. It stays upright on every edge and during a cross-screen drag.

At rest the tab shows a still version of the desktop subtitle overlay’s blue double ribbon. The first tap crossfades the ribbon into a play or stop symbol centered on the same surface. The second tap performs that action. After three idle seconds the ribbon returns. A drag also restores the ribbon until release. Transitions take 160 ms at normal speed, reverse from their current opacity, and complete immediately when system animations are disabled. The Android header retains the launcher artwork, and Quick Settings and notifications retain the five-bar tray mark.

The optional idle notification contains a Start action. During capture the service already owns an ongoing notification with Stop. Once the service releases its resources, the idle notification can return. Android controls channel availability, lock screen presentation and user dismissal. Turning off the idle notification preference never stops an active session.

Locking the device ends capture. When an active system-audio-only session ends because of the lock, the process remembers that interruption until unlock. If notifications are allowed, it offers one dismissible Continue action using the same notification slot. This works with the persistent idle notice disabled. Continuing requires fresh Android consent and a matching system-only input selection. An explicit Stop, a permission failure, or a session using the microphone never creates this reminder.

The tile, edge button and notifications belong to the Android adapter. Gemini connection rotation remains in the shared provider runtime. [Android Quick Settings](https://developer.android.com/develop/ui/views/quicksettings-tiles) · [Screen-sharing consent](https://developer.android.com/media/grow/media-projection#user_consent)

## Main-window hierarchy

More starts with the shortcut group: Add to Quick Settings is the first action, followed by the independent edge and idle-notification switches. Interface language is separate from Help and about. The guide, content-free diagnostics and installed version/update check share that lower group. The version and check action use one row when their measured labels fit and stack at narrow widths or larger text sizes.

The caption page keeps the selected service beside its input/language choices, before display, appearance and storage controls. More omits the idle start/setup footer; an unconfigured service page omits its duplicate setup action. Captions always retains the primary action, configured services can start directly, and every page retains session controls while starting, running or stopping. A settings-load failure exposes its existing recovery state. Page switching preserves scroll position and keyboard focus; it never starts capture or changes a shortcut preference.
