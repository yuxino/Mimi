package app.yuxino.mimi.android.shortcuts

import android.animation.ValueAnimator
import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.graphics.Color
import android.graphics.Rect
import android.hardware.display.DisplayManager
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.provider.Settings
import android.view.Display
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.ViewTreeObserver
import android.view.WindowManager
import android.view.inspector.WindowInspector
import app.yuxino.mimi.android.CaptureProbeActivity
import app.yuxino.mimi.android.MimiApplication
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.localization.InterfaceLanguage
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.roundToInt

/** Actual light-right screen pixels across the normal edge gestures; no capture or provider. */
internal class CaptionEdgeStabilityChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private val host get() = (context.applicationContext as MimiApplication).captionShortcuts
    private var bounds = Rect()
    private var heldSince: Long? = null
    private var stage = "setup"
    private fun dp(value: Int) = (value * context.resources.displayMetrics.density).roundToInt()
    private fun label(resource: Int) = InterfaceLanguage.context(context).getString(resource)
    private fun artwork(button: View) = (button as ViewGroup).getChildAt(0) as CaptionEdgeArtwork
    private fun badgeOpacity(art: CaptionEdgeArtwork): Int {
        val badge = art.getChildAt(1)
        return if (badge.isShown) (badge.alpha * 255).roundToInt() else 0
    }
    private fun ui(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        test.waitForIdleSync()
        failure?.let { throw it }
    }

    private fun edge(resource: Int): View {
        val finalOpacity = if (resource == R.string.shortcut_reveal_edge) 0 else 255
        val deadline = SystemClock.elapsedRealtime() + 5_000
        do {
            var found: View? = null
            ui {
                found = WindowInspector.getGlobalWindowViews().singleOrNull {
                    it.isShown && it.contentDescription == label(resource) && it.width == dp(48) && it.height == dp(48)
                }?.takeIf { badgeOpacity(artwork(it)) == finalOpacity }
            }
            found?.let { return it }
            SystemClock.sleep(25)
        } while (SystemClock.elapsedRealtime() < deadline)
        error("edge_stability_timeout: $stage")
    }

    private fun pixels(resource: Int): IntArray {
        val view = edge(resource)
        ui {
            val position = IntArray(2).also(view::getLocationOnScreen)
            val current = Rect(position[0], position[1], position[0] + view.width, position[1] + view.height)
            if (bounds.isEmpty) bounds = current
            check(current == bounds && current.right == context.resources.displayMetrics.widthPixels) { "edge_window_moved: $stage" }
        }
        SystemClock.sleep(80) // Let the observed state reach the native compositor.
        val screen = checkNotNull(test.uiAutomation.takeScreenshot())
        try {
            check(screen.getPixel(bounds.left - dp(8), bounds.centerY()) == LIGHT) { "edge_backdrop_is_not_fixed_light" }
            return IntArray(bounds.width() * bounds.height()).also {
                screen.getPixels(it, 0, bounds.width(), bounds.left, bounds.top, bounds.width(), bounds.height())
            }
        } finally { screen.recycle() }
    }

    private fun checkBadgeOnly(reference: IntArray, current: IntArray) {
        val changed = Rect()
        for (index in reference.indices) if (reference[index] != current[index]) {
            val x = index % bounds.width()
            val y = index / bounds.width()
            changed.union(x, y, x + 1, y + 1)
        }
        check(!changed.isEmpty) { "edge_action_badge_not_painted: $stage" }
        check(changed.left >= bounds.width() / 2 && changed.top >= bounds.height() / 2 &&
            changed.width() <= dp(16) && changed.height() <= dp(16)) {
            "edge_logo_flashed: $stage; changedBounds=$changed, maximumBadge=${dp(16)}px"
        }
    }

    private fun touch(action: Int) {
        val now = SystemClock.uptimeMillis()
        if (action == MotionEvent.ACTION_DOWN) heldSince = now
        val event = MotionEvent.obtain(checkNotNull(heldSince), now, action,
            bounds.exactCenterX(), bounds.exactCenterY(), 0).apply { source = InputDevice.SOURCE_TOUCHSCREEN }
        try { check(test.uiAutomation.injectInputEvent(event, true)) { "edge_touch_rejected" } }
        finally { event.recycle() }
        if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) heldSince = null
    }

    /** Retarget the same attached badge during a real draw, without estimating when a frame will run. */
    private fun checkBadgeTransition(): String {
        val button = edge(R.string.shortcut_reveal_edge)
        lateinit var art: CaptionEdgeArtwork
        ui { art = artwork(button) }
        if (!ValueAnimator.areAnimatorsEnabled()) {
            ui {
                art.showAction(true)
                check(badgeOpacity(art) == 255) { "disabled_animation_delayed_reveal" }
                art.showAction(false)
                check(badgeOpacity(art) == 0) { "disabled_animation_delayed_retuck" }
            }
            return "disabled: immediate"
        }

        val finished = CountDownLatch(1)
        var turningOpacity = -1
        var afterTurn = -1
        var fallingOpacity = -1
        var lastDraw = -1
        val draw = ViewTreeObserver.OnPreDrawListener {
            lastDraw = badgeOpacity(art)
            if (turningOpacity < 0 && lastDraw in 96..254) {
                turningOpacity = lastDraw
                art.showAction(false)
                afterTurn = badgeOpacity(art)
            } else if (lastDraw in 1 until turningOpacity) {
                fallingOpacity = lastDraw
            } else if ((turningOpacity >= 0 && lastDraw == 0) || lastDraw == 255) {
                finished.countDown()
            }
            true
        }
        try {
            ui {
                art.viewTreeObserver.addOnPreDrawListener(draw)
                art.showAction(true)
                check(badgeOpacity(art) == 0) { "edge_badge_flashed_on_reveal" }
            }
            val completed = finished.await(5, TimeUnit.SECONDS)
            ui {
                check(completed && afterTurn == turningOpacity && fallingOpacity > 0 && lastDraw == 0) {
                    "edge_badge_transition_incomplete: turn=$turningOpacity after=$afterTurn falling=$fallingOpacity last=$lastDraw"
                }
            }
            return "reversed at $turningOpacity/255, drew $fallingOpacity/255 on return"
        } finally {
            ui { art.viewTreeObserver.removeOnPreDrawListener(draw) }
        }
    }

    fun run() {
        var activity: CaptureProbeActivity? = null
        var window: CaptionEdgeWindow? = null
        var originalEdge: Boolean? = null
        var toggles = 0
        var badgeMotion = ""
        var failure: Throwable? = null
        try {
            check(Build.VERSION.SDK_INT >= 29 && Build.HARDWARE in setOf("ranchu", "goldfish")) { "Use an isolated emulator" }
            check(Settings.canDrawOverlays(context)) { "Grant the overlay permission before this test" }
            check(!MimiService.isRunning && !MimiService.isStarting && !MimiService.isStopping)
            ui { originalEdge = host.showEdge; host.showEdge = false }
            activity = test.startActivitySync(Intent(context, CaptureProbeActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as CaptureProbeActivity
            ui {
                val backdrop = checkNotNull(activity)
                backdrop.setContentView(View(backdrop).apply { setBackgroundColor(LIGHT) })
                val display = context.getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY)
                val windowContext = if (Build.VERSION.SDK_INT >= 30) context.createDisplayContext(checkNotNull(display))
                    .createWindowContext(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY, null) else context
                window = CaptionEdgeWindow(windowContext, CaptionEdgePlacement(1f, .5f), { toggles++ }, {}).also { it.show() }
            }
            stage = "tucked reference"
            val reference = pixels(R.string.shortcut_reveal_edge)
            check(reference.count { it != LIGHT } >= 32) { "edge_logo_not_painted" }
            stage = "first physical tap"
            touch(MotionEvent.ACTION_DOWN); SystemClock.sleep(70); touch(MotionEvent.ACTION_UP)
            checkBadgeOnly(reference, pixels(R.string.start_capture))
            stage = "held"
            touch(MotionEvent.ACTION_DOWN); SystemClock.sleep(120)
            checkBadgeOnly(reference, pixels(R.string.start_capture))
            val remaining = ViewConfiguration.getLongPressTimeout() + 100L -
                (SystemClock.uptimeMillis() - checkNotNull(heldSince))
            if (remaining > 0) SystemClock.sleep(remaining)
            touch(MotionEvent.ACTION_UP)
            val released = SystemClock.elapsedRealtime()
            stage = "released"
            checkBadgeOnly(reference, pixels(R.string.start_capture))
            stage = "three second retuck"
            val retucked = pixels(R.string.shortcut_reveal_edge)
            check(SystemClock.elapsedRealtime() - released in 2_500L..5_000L) { "edge_retuck_timing" }
            check(reference.contentEquals(retucked)) { "edge_retuck_changed_logo" }
            check(toggles == 0) { "edge_reveal_or_hold_activated_a_toggle" }
            stage = "badge transition"
            badgeMotion = checkBadgeTransition()
        } catch (error: Throwable) { failure = error }
        finally {
            try {
                if (heldSince != null) touch(MotionEvent.ACTION_CANCEL)
                ui { window?.close(); activity?.finish(); originalEdge?.let { host.showEdge = it } }
            } catch (error: Throwable) {
                if (failure == null) failure = error else failure.addSuppressed(error)
            }
        }
        val error = failure
        test.finish(if (error == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (error == null)
                "Caption edge stability passed: light-right; 48dp; badge-only changes; retuck identical; no toggle; $badgeMotion.\n"
            else "Caption edge stability failed at $stage: ${error.javaClass.simpleName}: ${error.message}\n")
        })
    }

    private companion object { val LIGHT: Int = Color.rgb(238, 238, 238) }
}
