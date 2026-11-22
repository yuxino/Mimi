package app.yuxino.mimi.android.shortcuts

import android.app.Activity
import android.app.Instrumentation
import android.app.NotificationManager
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.provider.Settings
import android.view.InputDevice
import android.view.MotionEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inspector.WindowInspector
import app.yuxino.mimi.android.MainActivity
import app.yuxino.mimi.android.MimiApplication
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.CaptureInput
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.capture.PlaybackCaptureChecks
import app.yuxino.mimi.android.localization.InterfaceLanguage
import app.yuxino.mimi.android.provider.ProviderEngine
import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.settings.SettingsStore
import java.io.File
import java.util.Locale
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

/** Runs on an empty API 35 emulator with its host microphone disabled. The OS owns every grant. */
internal class CaptionShortcutChecks(private val test: Instrumentation) {
    private val context = test.targetContext
    private val host get() = (context.applicationContext as MimiApplication).captionShortcuts
    private val notifications = context.getSystemService(NotificationManager::class.java)
    private val consent = PlaybackCaptureChecks(test)
    private val tileTitles by lazy {
        InterfaceLanguage.tags.filter(String::isNotEmpty).mapTo(mutableSetOf()) { tag ->
            val locale = Configuration(context.resources.configuration).apply { setLocale(Locale.forLanguageTag(tag)) }
            context.createConfigurationContext(locale).getString(R.string.shortcut_tile_label)
        }
    }
    private var stage = "prepare"
    private var evidence: File? = null

    fun run(args: Bundle?) {
        val chunks = AtomicLong()
        val nonzeroChunks = AtomicLong()
        val video = args?.getString("media_id")?.toLongOrNull()
        val noNotificationPermission = args?.getString("shortcut_without_notifications") == "true"
        val engine = object : ProviderEngine {
            override val sampleRateHz = 16_000
            override fun start() = Unit
            override fun sendAudio(pcm16Mono: ByteArray) {
                if (pcm16Mono.any { it != 0.toByte() }) nonzeroChunks.incrementAndGet()
                chunks.incrementAndGet()
            }
            override fun stop() = Unit
        }
        var profile: String? = null
        var edgeBeforePreferenceCheck: Boolean? = null
        val result = runCatching {
            check(Build.VERSION.SDK_INT >= 35 && args?.getString("host_microphone_disabled") == "true")
            // The instrumentation worker may start before MimiApplication.onCreate has returned.
            test.waitForIdleSync()
            consent.verifyEmulatorConsentMode()
            check(SettingsStore.profiles(context).isEmpty() && SettingsStore.captureInput(context) == CaptureInput.SYSTEM)
            check(!MimiService.isRunning && !MimiService.isStarting && !MimiService.isStopping)
            check(Settings.canDrawOverlays(context))
            if (noNotificationPermission) {
                check(video != null && !host.showEdge && !host.showNotification && !CaptionShortcutNotification.canPost(context) &&
                    context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                    "Use a fresh notification permission, both optional shortcuts off, an installed tile and a playback video"
                }
            } else {
                check(CaptionShortcutNotification.canPost(context) && host.showNotification && host.showEdge) {
                    "Enable both shortcuts and add the tile on the test device"
                }
            }
            if (args?.getString("review") == "true") evidence = context.getExternalFilesDir("shortcut-review")
            onMain {
                MimiService.setCaptureEngineForTests(context, engine)
                check(SettingsStore.saveConfiguration(context, ServiceConfiguration(ServiceProvider.DASHSCOPE,
                    mapOf("apiKey" to "offline-shortcut-fixture"))))
                check(SettingsStore.activateProvider(context, ServiceProvider.DASHSCOPE))
                profile = SettingsStore.activeProfile(context)?.id
            }
            stage = "ordinary launcher cannot start capture"
            test.startActivitySync(Intent(context, MainActivity::class.java).setAction(CaptionShortcutActions.START_CAPTIONS)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            await { activePackage() == context.packageName }
            check(!MimiService.isStarting && !MimiService.isRunning)
            if (video == null) context.startActivity(Intent(Settings.ACTION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            else shell("am start -W --user current -a android.intent.action.VIEW " +
                "-d content://media/external/video/media/$video -t video/mp4")
            await { activePackage()?.let { it != context.packageName && it != "com.android.systemui" } == true }
            val previousApp = checkNotNull(activePackage())

            stage = "cancel from tile"
            tile("01-tile-off")
            if (context.checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                var allow: AccessibilityNodeInfo? = null
                await {
                    val root = test.uiAutomation.rootInActiveWindow
                    if (root?.packageName?.endsWith("permissioncontroller") == true) {
                        allow = root.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_allow_foreground_only_button")
                            .firstOrNull { it.isVisibleToUser }
                    }
                    allow != null
                }
                shot("01a-audio-permission")
                tap(Rect().also { checkNotNull(allow).getBoundsInScreen(it) })
            }
            waitForConsent()
            shot("02-fresh-consent")
            consent.respondToEmulatorConsent(false)
            await { activePackage() == previousApp && !MimiService.ownUiVisible }
            check(!MimiService.isRunning && MimiService.captureProjectionForTests == null &&
                chunks.get() == 0L && nonzeroChunks.get() == 0L)
            check(hasNotification("caption_shortcuts") == !noNotificationPermission && !hasNotification("capture"))

            stage = "start from tile"
            tile()
            waitForConsent()
            consent.respondToEmulatorConsent(true)
            await { MimiService.isRunning && chunks.get() > 0 && MimiService.captureInput == CaptureInput.SYSTEM }
            if (video != null) {
                stage = "receive nonzero system audio from playback"
                await { nonzeroChunks.get() > 0 }
            }
            await { !MimiService.ownUiVisible && activePackage() == previousApp }
            if (noNotificationPermission) {
                stage = "tile starts playback capture without requesting notification permission"
                check(MimiService.captureProjectionForTests != null && nonzeroChunks.get() > 0 &&
                    context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED)
                shot("03-returned-with-notifications-off")
                stage = "stop from the tile without notification access"
                tile()
                await { !MimiService.isRunning && !MimiService.isStopping && MimiService.captureProjectionForTests == null }
                shell("cmd statusbar collapse")
                checkExplicitNotificationOffer()
                return@runCatching
            }
            await { hasNotification("capture") && !hasNotification("caption_shortcuts") }
            shot("03-returned-to-playback")
            if (args?.getString("edge_preference") == "true") {
                edgeBeforePreferenceCheck = host.showEdge
                checkEdgePreferenceDuringPlayback(previousApp, chunks)
            }

            stage = "stop from tile"
            tile("04-tile-on")
            stopped()
            stage = "close Quick Settings over playback"
            shell("cmd statusbar collapse")
            await { activePackage() == previousApp && readShell("dumpsys window displays").lineSequence()
                .any { "mCurrentFocus=" in it && previousApp in it } }

            stage = "wait for tucked edge"
            val tucked = label(R.string.shortcut_reveal_edge)
            val start = label(R.string.start_capture)
            await { edge(tucked) != null }
            val first = checkNotNull(edge(tucked))
            check(first.width() >= dp(48) && first.height() >= dp(48))
            stage = "release Quick Settings touch region over edge"
            await { shadeReleased(first) }
            stage = "first edge tap reveals"
            shot("edge-before-reveal")
            test.sendStatus(1, Bundle().apply { putString("stream", "Tucked edge bounds $first, previous app $previousApp\n") })
            tap(first)
            shot("edge-after-reveal")
            await { edge(start) != null }
            val before = checkNotNull(edge(start))
            val toX = if (before.centerX() > context.resources.displayMetrics.widthPixels / 2)
                context.resources.displayMetrics.widthPixels / 4 else context.resources.displayMetrics.widthPixels * 3 / 4
            stage = "move edge across screen"
            shell("input swipe ${before.centerX()} ${before.centerY()} $toX ${before.centerY()} 480")
            await { edge(start)?.let { kotlin.math.abs(it.centerX() - toX) < dp(12) } == true }
            shot("05-edge-dragged")
            stage = "tuck on opposite edge"
            await { edge(tucked)?.let { kotlin.math.abs(it.centerX() - first.centerX()) > dp(60) } == true }
            shot("06-edge-tucked")
            check(!MimiService.isStarting && !MimiService.isRunning && !MimiService.ownUiVisible)
            tap(checkNotNull(edge(tucked)))
            await { edge(start) != null }
            check(!MimiService.isStarting && !MimiService.isRunning && activePackage() == previousApp)
            shot("07-edge-expanded")
            stage = "start from the visible edge command"
            tap(edgeCommand(label(R.string.start_action)))
            waitForConsent()
            consent.respondToEmulatorConsent(true)
            await { MimiService.isRunning && activePackage() == previousApp }

            stage = "stop from notification"
            shell("cmd statusbar expand-notifications")
            tapSystem(label(R.string.stop_action), "08-notification-on")
            stopped()

            stage = "start from idle notification"
            shell("cmd statusbar expand-notifications")
            tapSystem(label(R.string.shortcut_captions_off), "09-notification-off")
            waitForConsent()
            consent.respondToEmulatorConsent(true)
            await { MimiService.isRunning && activePackage() == previousApp && !MimiService.ownUiVisible }

            stage = "stop from edge"
            await { edge(tucked) != null }
            tap(checkNotNull(edge(tucked)))
            val stop = label(R.string.stop_capture)
            await { edge(stop) != null && MimiService.isRunning }
            val character = checkNotNull(edge(stop))
            tap(character)
            val stopCommand = edgeCommand(label(R.string.stop_action))
            check(MimiService.isRunning && !MimiService.isStopping &&
                !stopCommand.contains(character.centerX(), character.centerY()))
            shot("17-edge-stop")
            tap(stopCommand)
            stopped()
            if (args?.getString("screen_lock_resume") == "true") checkResumeAfterUnlock(previousApp, chunks)
            check(chunks.get() > 0 && (video == null || nonzeroChunks.get() > 0))
        }
        if (result.isFailure && evidence != null) shot("failed-state")
        val cleanup = runCatching {
            if (profile != null) {
                if (MimiService.isRunning) onMain { context.startService(MimiService.stopIntent(context)) }
                await { !MimiService.isRunning && !MimiService.isStopping }
                onMain {
                    edgeBeforePreferenceCheck?.let { host.showEdge = it }
                    check(SettingsStore.deleteProfile(context, checkNotNull(profile)))
                }
            }
            onMain { MimiService.setCaptureEngineForTests(context, null) }
            shell("cmd statusbar collapse")
        }
        val error = result.exceptionOrNull() ?: cleanup.exceptionOrNull()
        test.finish(if (error == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            val counts = "Discarded audio chunks: ${chunks.get()}, nonzero: ${nonzeroChunks.get()}."
            val scenario = if (noNotificationPermission) "Tile playback without notification permission and explicit notification setting" else "Three native caption shortcuts"
            putString("stream", if (error == null) "$scenario passed. $counts\n"
                else "Caption shortcut check failed at $stage: ${error.message}. $counts\n")
        })
    }

    private fun label(id: Int) = InterfaceLanguage.context(context).getString(id)
    private fun onMain(block: () -> Unit) {
        var result: Result<Unit>? = null
        test.runOnMainSync { result = runCatching(block) }
        checkNotNull(result).getOrThrow()
    }
    private fun dp(value: Int) = (context.resources.displayMetrics.density * value + .5f).toInt()
    private fun activePackage() = test.uiAutomation.rootInActiveWindow?.packageName?.toString()
    private fun hasNotification(channel: String) = notifications.activeNotifications.any { it.notification.channelId == channel }
    private fun stopped() = await { !MimiService.isRunning && !MimiService.isStarting && !MimiService.isStopping &&
        MimiService.captureProjectionForTests == null && !hasNotification("capture") && hasNotification("caption_shortcuts") }

    private fun checkExplicitNotificationOffer() {
        stage = "open the optional notification setting"
        shell("am start -W --user current -a android.intent.action.MAIN -c android.intent.category.LAUNCHER " +
            "-f 0x10200000 -n ${context.packageName}/${MainActivity::class.java.name}")
        fun more() = appNode { it.viewIdResourceName == "${context.packageName}:id/tab_general" }
        await { more() != null && MimiService.ownUiVisible }
        check(checkNotNull(more()).performAction(AccessibilityNodeInfo.ACTION_CLICK))
        await { more()?.isChecked == true }
        awaitMorePanelDrawn()
        val name = label(R.string.shortcut_notification_label)
        fun setting() = appNode { it.isCheckable && it.text?.toString() == name }
        await { setting()?.isChecked == false }
        tapAfterWindowTransition(Rect().also { checkNotNull(setting()).getBoundsInScreen(it) })
        stage = "only the explicit setting opens notification permission"
        await {
            val dialog = test.uiAutomation.rootInActiveWindow
            dialog?.packageName?.endsWith("permissioncontroller") == true &&
                dialog.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_allow_button").any { it.isVisibleToUser }
        }
        shot("18-notification-permission-from-More")
        shell("input keyevent KEYCODE_BACK")
        await { setting()?.isChecked == false && !host.showNotification && !CaptionShortcutNotification.canPost(context) }
    }

    private fun checkEdgePreferenceDuringPlayback(player: String, chunks: AtomicLong) {
        val tucked = label(R.string.shortcut_reveal_edge)
        val stop = label(R.string.stop_capture)
        val projection = checkNotNull(MimiService.captureProjectionForTests)
        await { edge(tucked) != null }
        val restingBounds = checkNotNull(edge(tucked))
        val playerRecord = Regex("""topResumedActivity=ActivityRecord\{\S+ u\d+ ${Regex.escape(player)}/\S+ t(\d+)\b""")
        val playing = checkNotNull(playerRecord.find(readShell("dumpsys activity activities"))) { "Missing resumed playback task" }
        for (enabled in listOf(false, true)) {
            val change = if (enabled) "show" else "hide"
            stage = "open Mimi over playback to $change the floating button"
            shell("am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -f 0x10200000 " +
                "-n ${context.packageName}/${MainActivity::class.java.name}")
            fun more() = appNode { it.viewIdResourceName == "${context.packageName}:id/tab_general" }
            await { more() != null && MimiService.ownUiVisible }
            stage = "select More to $change the floating button"
            check(checkNotNull(more()).performAction(AccessibilityNodeInfo.ACTION_CLICK))
            await { more()?.isChecked == true }
            awaitMorePanelDrawn()
            val switchLabel = label(R.string.shortcut_edge_label)
            fun edgeSwitch() = appNode { it.isCheckable && it.text?.toString() == switchLabel }
            stage = "find the floating button switch before $change"
            await { edgeSwitch()?.isChecked == !enabled }
            val target = Rect().also { checkNotNull(edgeSwitch()).getBoundsInScreen(it) }
            test.sendStatus(1, Bundle().apply { putString("stream", "Physical edge switch tap ($change) at $target\n") })
            tapAfterWindowTransition(target)
            stage = "confirm the switch saved after $change"
            await { edgeSwitch()?.isChecked == enabled && host.showEdge == enabled }
            check(host.showNotification)
            shot(if (enabled) "15-edge-switch-on" else "13-edge-switch-off")

            stage = "return from More to playback after $change"
            shell("input keyevent KEYCODE_BACK")
            await { appNode { it.viewIdResourceName == "${context.packageName}:id/tab_caption" && it.isChecked } != null }
            // Select the already-open video in Recents. Back follows Mimi's own task history.
            shell("am task focus ${playing.groupValues[1]}")
            await { activePackage() == player && !MimiService.ownUiVisible &&
                readShell("dumpsys activity activities").contains(playing.value) }
            stage = "verify the floating button and live audio after $change"
            if (enabled) await { edge(tucked) == restingBounds }
            else check(edge(tucked) == null && edge(stop) == null)
            val received = chunks.get()
            await { chunks.get() > received }
            check(MimiService.isRunning && MimiService.captureProjectionForTests === projection &&
                MimiService.captureInput == CaptureInput.SYSTEM && hasNotification("capture") && !hasNotification("caption_shortcuts"))
            shot(if (enabled) "16-edge-restored-at-playback" else "14-playback-without-edge")
        }
    }

    /** Switching tabs removes the pinned preview and moves the whole scrolling settings surface. */
    private fun awaitMorePanelDrawn() {
        val committed = CountDownLatch(1)
        await {
            var scheduled = false
            onMain {
                val root = WindowInspector.getGlobalWindowViews().firstOrNull {
                    it.hasWindowFocus() && it.findViewById<android.widget.RadioButton>(R.id.tab_general)?.isChecked == true
                } ?: return@onMain
                root.viewTreeObserver.registerFrameCommitCallback { committed.countDown() }
                root.invalidate()
                scheduled = true
            }
            scheduled
        }
        check(committed.await(15, TimeUnit.SECONDS)) { "More page did not draw a frame" }
    }

    // An activity can be focused and drawn while its surface is still sliding over the player.
    // UiAutomation synchronizes window animations before sending the touch to Android's input dispatcher.
    private fun tapAfterWindowTransition(bounds: Rect) {
        val down = SystemClock.uptimeMillis()
        for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
            val touch = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action,
                bounds.exactCenterX(), bounds.exactCenterY(), 0)
            try {
                touch.source = InputDevice.SOURCE_TOUCHSCREEN
                check(test.uiAutomation.injectInputEvent(touch, true)) { "Android rejected the touch event" }
            } finally { touch.recycle() }
        }
    }

    private fun checkResumeAfterUnlock(previousApp: String, chunks: AtomicLong) {
        val power = context.getSystemService(android.os.PowerManager::class.java)
        val keyguard = context.getSystemService(android.app.KeyguardManager::class.java)
        check(!keyguard.isDeviceSecure) { "Use the blank emulator without a device credential" }
        onMain { host.showNotification = false }
        try {
            stage = "start system audio before locking"
            tile()
            waitForConsent()
            consent.respondToEmulatorConsent(true)
            await { MimiService.isRunning && activePackage() == previousApp && hasNotification("capture") }
            stage = "lock stops capture without showing a shortcut"
            shell("input keyevent KEYCODE_SLEEP")
            await { !power.isInteractive && !MimiService.isRunning && !MimiService.isStopping &&
                MimiService.captureProjectionForTests == null && !hasNotification("capture") }
            check(!hasNotification("caption_shortcuts"))
            val stoppedAt = chunks.get()
            stage = "unlock offers resume without starting capture"
            shell("input keyevent KEYCODE_WAKEUP")
            shell("wm dismiss-keyguard")
            try {
                await { power.isInteractive && !keyguard.isKeyguardLocked && hasNotification("caption_shortcuts") }
            } catch (timeout: IllegalStateException) {
                var diagnostic = ""
                onMain {
                    // Read only on failure in the unminified debug APK. Keep the private
                    // continuation out of the product API and leave the original timing alone.
                    val pending = runCatching {
                        val field = host.javaClass.getDeclaredField("continuation").apply { isAccessible = true }
                        (field.get(host) as CaptionUnlockContinuation).pending.toString()
                    }.getOrDefault("unavailable")
                    diagnostic = "interactive=${power.isInteractive},keyguard=${keyguard.isKeyguardLocked}," +
                        "deviceLocked=${keyguard.isDeviceLocked},canPost=${CaptionShortcutNotification.canPost(context)}," +
                        "shortcutsNotice=${hasNotification("caption_shortcuts")},idleOptIn=${host.showNotification}," +
                        "pending=$pending,running=${MimiService.isRunning},lastCaptureError=${MimiService.lastCaptureError}"
                }
                throw IllegalStateException("${timeout.message}; $diagnostic", timeout)
            }
            check(!MimiService.isRunning && !MimiService.isStarting && chunks.get() == stoppedAt)
            shell("cmd statusbar expand-notifications")
            tapSystem(label(R.string.shortcut_resume_captions), "10-unlock-resume")
            waitForConsent()
            check(MimiService.captureProjectionForTests == null && chunks.get() == stoppedAt)
            shot("11-resume-fresh-consent")
            stage = "resume creates a fresh capture after approval"
            consent.respondToEmulatorConsent(true)
            await { MimiService.isRunning && chunks.get() > stoppedAt && activePackage() == previousApp }
            shot("12-resumed-playback")
            tile()
            await { !MimiService.isRunning && !MimiService.isStopping && !hasNotification("capture") }
            check(!hasNotification("caption_shortcuts")) { "An explicit stop must not offer resume" }
        } finally {
            shell("input keyevent KEYCODE_WAKEUP")
            shell("wm dismiss-keyguard")
            onMain { host.showNotification = true }
        }
    }

    private fun waitForConsent() = await { systemNode { it.viewIdResourceName?.endsWith("/screen_share_mode_spinner") == true } != null }

    private fun tile(image: String? = null) {
        shell("cmd statusbar expand-settings")
        fun pager() = systemNode { it.viewIdResourceName?.endsWith("/qs_pager") == true }
        fun page() = systemNode { it.viewIdResourceName?.endsWith("/footer_page_indicator") == true }?.contentDescription
        // SystemUI can leave only the compact notification shade open. A second pull opens its pager.
        val settleMs = context.resources.getInteger(android.R.integer.config_mediumAnimTime)
        var compactSince = 0L
        var pulled = false
        await {
            val expanded = pager() != null
            if (!expanded && !pulled) {
                val compact = systemNode { it.viewIdResourceName?.endsWith("/quick_qs_panel") == true }
                val now = SystemClock.elapsedRealtime()
                when {
                    compact == null -> compactSince = 0L
                    compactSince == 0L -> compactSince = now
                    now - compactSince >= settleMs -> {
                        val bounds = Rect().also(compact::getBoundsInScreen)
                        val x = bounds.centerX()
                        val endY = context.resources.displayMetrics.heightPixels * 3 / 4
                        shell("input swipe $x ${bounds.top + dp(16)} $x $endY 440")
                        pulled = true
                    }
                }
            }
            expanded && page() != null
        }
        val name = label(R.string.shortcut_tile_label)
        // SystemUI may first draw the previous language on this page while the tile service binds.
        // Stay on the installed tile's page; tapSystem still waits for the exact current language.
        fun tileVisible() = systemNode { it.text?.toString() in tileTitles } != null
        // SystemUI retains its last page. Navigate the actual pager and wait for each settled page.
        for (direction in listOf(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD, AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD)) {
            while (!tileVisible()) {
                val visiblePager = checkNotNull(pager())
                if (visiblePager.actionList.none { it.id == direction }) break
                val previous = page()
                check(visiblePager.performAction(direction)) { "Quick Settings refused to change page" }
                await { page()?.let { it != previous } == true }
            }
            if (tileVisible()) return tapSystem(name, image)
        }
        error("installed tile is missing from Quick Settings")
    }

    private fun tapSystem(text: String, image: String? = null) {
        fun target() = systemNode { it.text?.toString().equals(text, true) ||
            it.contentDescription?.toString()?.split(',')?.any { word -> word.trim().equals(text, true) } == true }
        await { target() != null }
        image?.let(::shot)
        tap(Rect().also { checkNotNull(target()).getBoundsInScreen(it) })
    }

    private fun appNode(matches: (AccessibilityNodeInfo) -> Boolean) = visibleNode(context.packageName, matches)
    private fun systemNode(matches: (AccessibilityNodeInfo) -> Boolean) = visibleNode("com.android.systemui", matches)
    private fun visibleNode(packageName: String, matches: (AccessibilityNodeInfo) -> Boolean): AccessibilityNodeInfo? {
        val root = test.uiAutomation.rootInActiveWindow ?: return null
        if (root.packageName?.toString() != packageName) return null
        fun find(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
            if (node.isVisibleToUser && matches(node)) return node
            return (0 until node.childCount).firstNotNullOfOrNull { node.getChild(it)?.let(::find) }
        }
        return find(root)
    }

    private fun edge(description: String): Rect? {
        var bounds: Rect? = null
        onMain {
            bounds = CaptionShortcutWindows.character(description, dp(48))?.let(CaptionShortcutWindows::bounds)
        }
        return bounds
    }

    private fun edgeCommand(label: String): Rect {
        var bounds: Rect? = null
        await {
            onMain { bounds = CaptionShortcutWindows.command(label)?.let(CaptionShortcutWindows::bounds) }
            bounds != null
        }
        return checkNotNull(bounds)
    }

    /** Android returns focus to the player before SystemUI releases its full-screen touch region. */
    private fun shadeReleased(edge: Rect): Boolean {
        val windows = readShell("dumpsys input").lineSequence().filter { "displayId=0," in it && "touchableRegion=" in it }.toList()
        val edgeName = " ${label(R.string.shortcut_edge_label)},"
        if (windows.none { edgeName in it }) return false
        val shade = windows.firstOrNull { " NotificationShade," in it } ?: return true
        if ("NOT_VISIBLE" in shade || "NOT_TOUCHABLE" in shade) return true
        val region = shade.substringAfter("touchableRegion=").substringBefore(", ownerPid=")
        if (region == "<empty>") return true
        val rects = Regex("""\[(-?\d+),(-?\d+)]\[(-?\d+),(-?\d+)]""").findAll(region).map { match ->
            val (left, top, right, bottom) = match.destructured
            Rect(left.toInt(), top.toInt(), right.toInt(), bottom.toInt())
        }.toList()
        return rects.isNotEmpty() && rects.none { it.contains(edge.centerX(), edge.centerY()) }
    }

    private fun tap(rect: Rect) = shell("input tap ${rect.centerX()} ${rect.centerY()}")
    private fun shell(command: String) { readShell(command) }
    private fun readShell(command: String): String = ParcelFileDescriptor.AutoCloseInputStream(test.uiAutomation.executeShellCommand(command))
        .bufferedReader().use { it.readText() }
    private fun await(condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 15_000
        while (!condition()) {
            check(SystemClock.elapsedRealtime() < deadline) {
                val selectedTab = appNode { it.isChecked && it.className == "android.widget.RadioButton" }
                "timed out (foreground=${activePackage()}, ownUi=${MimiService.ownUiVisible}, tab=${selectedTab?.viewIdResourceName})"
            }
            SystemClock.sleep(75)
        }
    }
    private fun shot(name: String) {
        val directory = evidence ?: return
        SystemClock.sleep(350)
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        try { File(directory, "$name.png").outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) } }
        finally { bitmap.recycle() }
    }
}
