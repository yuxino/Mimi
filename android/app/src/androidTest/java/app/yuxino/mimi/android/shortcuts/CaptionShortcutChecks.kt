package app.yuxino.mimi.android.shortcuts

import android.app.Activity
import android.app.Instrumentation
import android.app.NotificationManager
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.provider.Settings
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
import java.util.concurrent.atomic.AtomicLong

/** Runs on an empty API 35 emulator with its host microphone disabled. The OS owns every grant. */
internal class CaptionShortcutChecks(private val test: Instrumentation) {
    private val context = test.targetContext
    private val host get() = (context.applicationContext as MimiApplication).captionShortcuts
    private val notifications = context.getSystemService(NotificationManager::class.java)
    private val consent = PlaybackCaptureChecks(test)
    private var stage = "prepare"
    private var evidence: File? = null

    fun run(args: Bundle?) {
        val chunks = AtomicLong()
        val nonzeroChunks = AtomicLong()
        val video = args?.getString("media_id")?.toLongOrNull()
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
        val result = runCatching {
            check(Build.VERSION.SDK_INT >= 35 && args?.getString("host_microphone_disabled") == "true")
            consent.verifyEmulatorConsentMode()
            check(SettingsStore.profiles(context).isEmpty() && SettingsStore.captureInput(context) == CaptureInput.SYSTEM)
            check(!MimiService.isRunning && !MimiService.isStarting && !MimiService.isStopping)
            check(Settings.canDrawOverlays(context) && CaptionShortcutNotification.canPost(context))
            check(host.showNotification && host.showEdge) { "Enable both shortcuts and add the tile on the test device" }
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
            else shell("am start -W -a android.intent.action.VIEW -d content://media/external/video/media/$video " +
                "-t video/mp4 --grant-read-uri-permission")
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
            check(hasNotification("caption_shortcuts") && !hasNotification("capture"))

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
            await { hasNotification("capture") && !hasNotification("caption_shortcuts") }
            shot("03-returned-to-playback")

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
            tap(checkNotNull(edge(start)))
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
            tap(checkNotNull(edge(stop)))
            stopped()
            if (args?.getString("screen_lock_resume") == "true") checkResumeAfterUnlock(previousApp, chunks)
            check(chunks.get() > 0 && (video == null || nonzeroChunks.get() > 0))
        }
        if (result.isFailure && evidence != null) shot("failed-state")
        val cleanup = runCatching {
            if (profile != null) {
                if (MimiService.isRunning) onMain { context.startService(MimiService.stopIntent(context)) }
                await { !MimiService.isRunning && !MimiService.isStopping }
                onMain { check(SettingsStore.deleteProfile(context, checkNotNull(profile))) }
            }
            onMain { MimiService.setCaptureEngineForTests(context, null) }
        }
        val error = result.exceptionOrNull() ?: cleanup.exceptionOrNull()
        test.finish(if (error == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            val counts = "Discarded audio chunks: ${chunks.get()}, nonzero: ${nonzeroChunks.get()}."
            putString("stream", if (error == null) "Three native caption shortcuts passed. $counts\n"
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
            await { power.isInteractive && !keyguard.isKeyguardLocked && hasNotification("caption_shortcuts") }
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
        await { systemNode { it.viewIdResourceName?.endsWith("/tile_label") == true } != null }
        val name = label(R.string.shortcut_tile_label)
        val metrics = context.resources.displayMetrics
        for (page in 0..5) {
            if (systemNode { it.text?.toString() == name } != null) {
                tapSystem(name, image)
                return
            }
            shell("input swipe ${metrics.widthPixels * 4 / 5} ${metrics.heightPixels / 2} " +
                "${metrics.widthPixels / 5} ${metrics.heightPixels / 2} 300")
            SystemClock.sleep(300)
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

    private fun systemNode(matches: (AccessibilityNodeInfo) -> Boolean): AccessibilityNodeInfo? {
        val root = test.uiAutomation.rootInActiveWindow ?: return null
        if (root.packageName != "com.android.systemui") return null
        fun find(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
            if (node.isVisibleToUser && matches(node)) return node
            return (0 until node.childCount).firstNotNullOfOrNull { node.getChild(it)?.let(::find) }
        }
        return find(root)
    }

    private fun edge(description: String): Rect? {
        var bounds: Rect? = null
        onMain {
            val view = WindowInspector.getGlobalWindowViews().firstOrNull { it.isShown && it.contentDescription == description }
            if (view != null) {
                val p = IntArray(2).also(view::getLocationOnScreen)
                bounds = Rect(p[0], p[1], p[0] + view.width, p[1] + view.height)
            }
        }
        return bounds
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
            check(SystemClock.elapsedRealtime() < deadline) { "timed out" }
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
