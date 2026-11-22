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
import kotlin.math.abs
import kotlin.math.roundToInt

/** Actual light-right screen pixels across the normal edge gestures; no capture or provider. */
internal class CaptionEdgeStabilityChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private val host get() = (context.applicationContext as MimiApplication).captionShortcuts
    private var bounds = Rect()
    private var heldSince: Long? = null
    private var stage = "setup"
    private val touchTrace = mutableListOf<String>()
    private fun dp(value: Int) = (value * context.resources.displayMetrics.density).roundToInt()
    private fun label(resource: Int) = InterfaceLanguage.context(context).getString(resource)
    private fun artwork(button: View) = (button as ViewGroup).getChildAt(0) as CaptionEdgeArtwork
    private fun actionOpacity(art: CaptionEdgeArtwork) = (art.actionOpacity * 255).roundToInt()
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
                }?.takeIf { actionOpacity(artwork(it)) == finalOpacity }
            }
            found?.let { return it }
            SystemClock.sleep(25)
        } while (SystemClock.elapsedRealtime() < deadline)
        error("edge_stability_timeout: $stage")
    }

    private fun frame(view: View): Rect = Rect().also { rect ->
        ui {
            val position = IntArray(2).also(view::getLocationOnScreen)
            rect.set(position[0], position[1], position[0] + view.width, position[1] + view.height)
        }
    }

    private fun pixels(resource: Int): IntArray {
        val current = frame(edge(resource))
        if (bounds.isEmpty) bounds = current
        check(current == bounds && current.right == context.resources.displayMetrics.widthPixels) { "edge_window_moved: $stage" }
        return pixels(current)
    }

    private fun pixels(frame: Rect): IntArray {
        SystemClock.sleep(80) // Let the observed state reach the native compositor.
        val screen = checkNotNull(test.uiAutomation.takeScreenshot())
        try {
            val backdrop = if (frame.left >= dp(8)) frame.left - dp(8) else frame.right + dp(8)
            check(screen.getPixel(backdrop, frame.centerY()) == LIGHT) { "edge_backdrop_is_not_fixed_light" }
            return IntArray(frame.width() * frame.height()).also {
                screen.getPixels(it, 0, frame.width(), frame.left, frame.top, frame.width(), frame.height())
            }
        } finally { screen.recycle() }
    }

    private fun tuckedPortrait(reference: IntArray): Rect {
        val portrait = Rect()
        var painted = 0
        for (index in reference.indices) if (reference[index] != LIGHT) {
            val x = index % bounds.width()
            val y = index / bounds.width()
            portrait.union(x, y, x + 1, y + 1)
            painted++
        }
        check(portrait.width() in dp(28)..dp(32) && portrait.height() in dp(28)..dp(32) &&
            painted >= portrait.width() * portrait.height() / 2) { "docked_edge_portrait_missing: $portrait" }
        return portrait
    }

    private fun checkInwardActionOnly(before: IntArray, after: IntArray, portrait: Rect, dockedRight: Boolean = true) {
        // A wing can meet the antialiased inner rim. The portrait interior and the
        // screen-edge side must keep their exact screenshot pixels throughout.
        val innerRadius = minOf(portrait.width(), portrait.height()) / 2f - dp(1)
        var changed = false
        for (index in before.indices) if (before[index] != after[index]) {
            val x = index % bounds.width()
            val y = index / bounds.width()
            val dx = x + .5f - portrait.exactCenterX()
            val dy = y + .5f - portrait.exactCenterY()
            val inward = if (dockedRight) x < bounds.width() / 2 else x >= bounds.width() / 2
            check(inward && y in portrait.top until portrait.bottom &&
                dx * dx + dy * dy >= innerRadius * innerRadius) {
                "edge_portrait_or_outer_rim_changed: $stage; pixel=($x,$y), portrait=$portrait"
            }
            changed = true
        }
        check(changed) { "inward_edge_action_not_painted: $stage" }
    }

    private fun touch(action: Int, x: Float = bounds.exactCenterX()) {
        val now = SystemClock.uptimeMillis()
        if (action == MotionEvent.ACTION_DOWN) heldSince = now
        val event = MotionEvent.obtain(checkNotNull(heldSince), now, action,
            x, bounds.exactCenterY(), 0).apply { source = InputDevice.SOURCE_TOUCHSCREEN }
        try { check(test.uiAutomation.injectInputEvent(event, true)) { "edge_touch_rejected" } }
        finally { event.recycle() }
        touchTrace.add("${MotionEvent.actionToString(action)}($x,${bounds.exactCenterY()})@$now")
        if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) heldSince = null
    }

    private fun checkDragAcrossDisplay() {
        touch(MotionEvent.ACTION_DOWN); touch(MotionEvent.ACTION_UP)
        val button = edge(R.string.start_capture)
        touch(MotionEvent.ACTION_DOWN)
        var fingerX = bounds.exactCenterX()
        fun moveAndHold(x: Float): Rect {
            val deadline = SystemClock.elapsedRealtime() + 2_000
            // A physical drag delivers a path and then stationary samples while held.
            // Keep the final center assertion independent of the injected coordinates.
            for (step in 1..8) {
                touch(MotionEvent.ACTION_MOVE, fingerX + (x - fingerX) * step / 8)
                SystemClock.sleep(16)
            }
            fingerX = x
            touch(MotionEvent.ACTION_MOVE, x)
            SystemClock.sleep(16)
            touch(MotionEvent.ACTION_MOVE, x)
            val observations = mutableListOf<String>()
            do {
                val current = frame(button)
                var hidden = false
                ui {
                    val params = button.layoutParams as WindowManager.LayoutParams
                    val opacity = actionOpacity(artwork(button))
                    hidden = opacity == 0
                    val state = "frame=$current,params=(${params.x},${params.y}),opacity=$opacity," +
                        "pressed=${button.isPressed},label=${button.contentDescription}"
                    if (observations.lastOrNull() != state) observations.add(state)
                }
                if (abs(current.exactCenterX() - x) <= 1 && hidden) return current
                SystemClock.sleep(20)
            } while (SystemClock.elapsedRealtime() < deadline)
            error("dragged_edge_or_action_did_not_settle_at_x=$x; observed=$observations; inputs=${touchTrace.takeLast(5)}")
        }
        stage = "held at display midpoint"
        val middle = moveAndHold(bounds.right / 2f)
        tuckedPortrait(pixels(middle))
        stage = "held near left edge"
        val dropX = bounds.width().toFloat() // Leave room for a real idle snap.
        val drop = moveAndHold(dropX)
        val dragged = pixels(drop)
        val portrait = tuckedPortrait(dragged)
        check(portrait.centerX() < drop.width() / 2)
        stage = "released near left edge"
        touch(MotionEvent.ACTION_UP, dropX)
        val reopened = frame(edge(R.string.start_capture))
        check(reopened == drop) { "edge_moved_on_drag_release" }
        checkInwardActionOnly(dragged, pixels(reopened), portrait, dockedRight = false)
        stage = "docked at left edge"
        val docked = frame(edge(R.string.shortcut_reveal_edge))
        check(drop.left > docked.left && docked.left == 0 && docked.top == bounds.top) { "drag_did_not_dock_left: $docked" }
        tuckedPortrait(pixels(docked))
    }

    /** Reverse the action during a real draw, without estimating when a frame will run. */
    private fun checkActionTransition(): String {
        val button = edge(R.string.shortcut_reveal_edge)
        lateinit var art: CaptionEdgeArtwork
        ui { art = artwork(button) }
        if (!ValueAnimator.areAnimatorsEnabled()) {
            ui {
                art.showAction(true)
                check(actionOpacity(art) == 255) { "disabled_animation_delayed_reveal" }
                art.showAction(false)
                check(actionOpacity(art) == 0) { "disabled_animation_delayed_retuck" }
            }
            return "disabled: immediate"
        }

        val finished = CountDownLatch(1)
        var turningOpacity = -1
        var afterTurn = -1
        var fallingOpacity = -1
        var lastDraw = -1
        val draw = ViewTreeObserver.OnPreDrawListener {
            lastDraw = actionOpacity(art)
            if (turningOpacity < 0 && lastDraw in 96..254) {
                turningOpacity = lastDraw
                art.showAction(false)
                afterTurn = actionOpacity(art)
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
                check(actionOpacity(art) == 0) { "edge_action_flashed_on_reveal" }
            }
            val completed = finished.await(5, TimeUnit.SECONDS)
            ui {
                check(completed && afterTurn == turningOpacity && fallingOpacity > 0 && lastDraw == 0) {
                    "edge_action_transition_incomplete: turn=$turningOpacity after=$afterTurn falling=$fallingOpacity last=$lastDraw"
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
        var actionMotion = ""
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
            val portrait = tuckedPortrait(reference)
            check(portrait.left > bounds.width() / 4 && portrait.contains(bounds.width() - dp(8), bounds.height() / 2))
            stage = "first physical tap"
            touch(MotionEvent.ACTION_DOWN); SystemClock.sleep(70); touch(MotionEvent.ACTION_UP)
            val expanded = pixels(R.string.start_capture)
            checkInwardActionOnly(reference, expanded, portrait)
            stage = "held"
            touch(MotionEvent.ACTION_DOWN); SystemClock.sleep(120)
            check(pixels(R.string.start_capture).contentEquals(expanded)) { "holding_edge_changed_artwork" }
            stage = "stop and play artwork while held"
            val button = edge(R.string.start_capture)
            ui { artwork(button).setStop(true) }
            checkInwardActionOnly(expanded, pixels(R.string.start_capture), portrait)
            ui { artwork(button).setStop(false) }
            val remaining = ViewConfiguration.getLongPressTimeout() + 100L -
                (SystemClock.uptimeMillis() - checkNotNull(heldSince))
            if (remaining > 0) SystemClock.sleep(remaining)
            touch(MotionEvent.ACTION_UP)
            val released = SystemClock.elapsedRealtime()
            stage = "released"
            check(pixels(R.string.start_capture).contentEquals(expanded)) { "released_edge_changed_artwork" }
            stage = "three second retuck"
            val retucked = pixels(R.string.shortcut_reveal_edge)
            check(SystemClock.elapsedRealtime() - released in 2_500L..5_000L) { "edge_retuck_timing" }
            check(reference.contentEquals(retucked)) { "edge_retuck_changed_logo" }
            check(toggles == 0) { "edge_reveal_or_hold_activated_a_toggle" }
            stage = "action transition"
            actionMotion = checkActionTransition()
            stage = "drag from revealed right edge"
            checkDragAcrossDisplay()
            check(toggles == 0) { "drag_activated_a_toggle" }
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
                "Caption edge stability passed: light-right; 48dp; fixed portrait; inward play/stop; retuck identical; drag across to left; no toggle; $actionMotion.\n"
            else "Caption edge stability failed at $stage: ${error.javaClass.simpleName}: ${error.message}\n")
        })
    }

    private companion object { val LIGHT: Int = Color.rgb(238, 238, 238) }
}
