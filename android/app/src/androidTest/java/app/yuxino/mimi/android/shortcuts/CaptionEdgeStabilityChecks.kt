package app.yuxino.mimi.android.shortcuts

import android.animation.ValueAnimator
import android.app.Activity
import android.app.Instrumentation
import android.content.Context
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

    private fun pixelBounds(pixels: IntArray, selected: (Int) -> Boolean): Rect {
        val result = Rect()
        for (index in pixels.indices) if (selected(pixels[index])) {
            val x = index % bounds.width()
            val y = index / bounds.width()
            result.union(x, y, x + 1, y + 1)
        }
        return result
    }

    private fun tabBounds(pixels: IntArray): Rect {
        val tab = pixelBounds(pixels) { it != LIGHT }
        check(tab.width() in dp(25)..dp(30) && tab.height() in dp(24)..dp(28)) {
            "edge_tab_size: $stage, $tab"
        }
        return tab
    }

    private fun checkDocked(pixels: IntArray, side: CaptionEdge) {
        val tab = tabBounds(pixels)
        for (offset in -dp(5)..dp(5)) {
            val x = when (side) {
                CaptionEdge.LEFT -> 0
                CaptionEdge.RIGHT -> bounds.width() - 1
                else -> tab.centerX() + offset
            }
            val y = when (side) {
                CaptionEdge.TOP -> 0
                CaptionEdge.BOTTOM -> bounds.height() - 1
                else -> tab.centerY() + offset
            }
            check(pixels[y * bounds.width() + x] != LIGHT) { "edge_tab_does_not_meet_$side: $stage, $tab" }
        }
    }

    // Include antialiasing and rounding around the central 18 x 14.4dp artwork.
    private fun markArea(tab: Rect) = Rect(
        tab.centerX() - dp(10), tab.centerY() - dp(8),
        tab.centerX() + dp(10) + 1, tab.centerY() + dp(8) + 1,
    )

    private fun waveInk(color: Int) = Color.blue(color) > 120 && Color.blue(color) - Color.red(color) > 35

    private fun waveBounds(pixels: IntArray): Rect {
        val wave = pixelBounds(pixels, ::waveInk)
        check(wave.width() >= dp(11) && wave.height() >= dp(4) && markArea(tabBounds(pixels)).contains(wave)) {
            "edge_wave_missing_or_misplaced: $stage, $wave"
        }
        return wave
    }

    private fun checkOnlyMarkChanged(before: IntArray, after: IntArray) {
        val tab = tabBounds(before)
        val mark = markArea(tab)
        check(tab == tabBounds(after)) { "edge_tab_outline_changed: $stage" }
        var changed = false
        for (index in before.indices) if (before[index] != after[index]) {
            val x = index % bounds.width()
            val y = index / bounds.width()
            check(mark.contains(x, y)) { "edge_pixels_changed_outside_mark: $stage, ($x,$y), mark=$mark" }
            changed = true
        }
        check(changed) { "edge_mark_did_not_change: $stage" }
    }

    private fun checkActionShape(pixels: IntArray, stop: Boolean) {
        val mark = markArea(tabBounds(pixels))
        check(pixels.none(::waveInk)) { "edge_wave_still_visible_behind_action: $stage" }
        fun bright(x: Int, y: Int): Boolean {
            val color = pixels[y * bounds.width() + x]
            return Color.red(color) > 155 && abs(Color.red(color) - Color.blue(color)) < 25 && color != LIGHT
        }
        val glyph = Rect()
        for (y in mark.top until mark.bottom) for (x in mark.left until mark.right) {
            if (bright(x, y)) glyph.union(x, y, x + 1, y + 1)
        }
        check(glyph.width() >= dp(5) && glyph.height() >= dp(5)) { "edge_action_too_small: $stage, $glyph" }
        fun heightAt(x: Int) = (glyph.top until glyph.bottom).count { bright(x, it) }
        val left = heightAt(glyph.left + glyph.width() / 4)
        val right = heightAt(glyph.right - 1 - glyph.width() / 4)
        // A right-facing play triangle tapers towards its tip on either side of the screen.
        val correct = if (stop) {
            minOf(left, right) >= glyph.height() * 3 / 4 && abs(glyph.width() - glyph.height()) <= dp(2)
        } else {
            left - right >= glyph.height() / 4
        }
        check(correct) { "edge_action_shape: $stage, stop=$stop glyph=$glyph columns=($left,$right)" }
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
        stage = "held across display midpoint"
        val middle = bounds.right / 2f
        val approaching = pixels(moveAndHold(middle + 1f))
        val dropX = bounds.width().toFloat() // Leave room for a real idle snap.
        val drop: Rect
        val dragged: IntArray
        var peakOpacity = 0
        val watchDrag = ViewTreeObserver.OnPreDrawListener {
            peakOpacity = maxOf(peakOpacity, actionOpacity(artwork(button)))
            true
        }
        ui { button.viewTreeObserver.addOnPreDrawListener(watchDrag) }
        try {
            val midpoint = pixels(moveAndHold(middle))
            val crossed = pixels(moveAndHold(middle - 1f))
            val first = waveBounds(approaching)
            for (sample in listOf(midpoint, crossed)) {
                val next = waveBounds(sample)
                check(abs(first.exactCenterX() - next.exactCenterX()) <= dp(1) &&
                    abs(first.exactCenterY() - next.exactCenterY()) <= dp(1)) {
                    "edge_mark_jumped_crossing_display: $first to $next"
                }
            }
            stage = "held near left edge"
            drop = moveAndHold(dropX)
            dragged = pixels(drop)
            waveBounds(dragged)
        } finally {
            ui { button.viewTreeObserver.removeOnPreDrawListener(watchDrag) }
        }
        check(peakOpacity == 0) { "edge_action_flashed_during_drag: $peakOpacity/255" }
        stage = "released near left edge"
        touch(MotionEvent.ACTION_UP, dropX)
        val reopened = frame(edge(R.string.start_capture))
        check(reopened == drop) { "edge_moved_on_drag_release" }
        val showingPlay = pixels(reopened)
        checkOnlyMarkChanged(dragged, showingPlay)
        checkActionShape(showingPlay, stop = false)
        stage = "docked at left edge"
        val docked = frame(edge(R.string.shortcut_reveal_edge))
        check(drop.left > docked.left && docked.left == 0 && docked.top == bounds.top) { "drag_did_not_dock_left: $docked" }
        val idle = pixels(docked)
        checkDocked(idle, CaptionEdge.LEFT)
        waveBounds(idle)
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
        lateinit var windowContext: Context
        var originalEdge: Boolean? = null
        var toggles = 0
        var actionMotion = ""
        var failure: Throwable? = null
        fun openWindow(placement: CaptionEdgePlacement) {
            window?.close()
            window = CaptionEdgeWindow(windowContext, placement, { toggles++ }, {}).also { it.show() }
        }
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
                windowContext = if (Build.VERSION.SDK_INT >= 30) context.createDisplayContext(checkNotNull(display))
                    .createWindowContext(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY, null) else context
                openWindow(CaptionEdgePlacement(1f, .5f))
            }
            stage = "tucked reference"
            val reference = pixels(R.string.shortcut_reveal_edge)
            waveBounds(reference)
            checkDocked(reference, CaptionEdge.RIGHT)
            SystemClock.sleep(270)
            check(reference.contentEquals(pixels(R.string.shortcut_reveal_edge))) { "idle_edge_pulsed" }
            stage = "first physical tap"
            touch(MotionEvent.ACTION_DOWN); SystemClock.sleep(70); touch(MotionEvent.ACTION_UP)
            val expanded = pixels(R.string.start_capture)
            checkOnlyMarkChanged(reference, expanded)
            checkActionShape(expanded, stop = false)
            stage = "held"
            touch(MotionEvent.ACTION_DOWN); SystemClock.sleep(120)
            check(pixels(R.string.start_capture).contentEquals(expanded)) { "holding_edge_changed_artwork" }
            stage = "stop and play artwork while held"
            val button = edge(R.string.start_capture)
            ui { artwork(button).setStop(true) }
            val stopped = pixels(R.string.start_capture)
            checkOnlyMarkChanged(expanded, stopped)
            checkActionShape(stopped, stop = true)
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
            for (side in listOf(CaptionEdge.TOP, CaptionEdge.BOTTOM)) {
                stage = "upright play on $side"
                ui { openWindow(CaptionEdgePlacement(.5f, if (side == CaptionEdge.TOP) 0f else 1f)) }
                val current = frame(edge(R.string.start_capture))
                check(if (side == CaptionEdge.TOP) current.bottom < bounds.top else current.top > bounds.bottom) {
                    "edge_window_did_not_move_to_$side: $current"
                }
                val play = pixels(current)
                checkDocked(play, side)
                checkActionShape(play, stop = false)
                val wave = pixels(frame(edge(R.string.shortcut_reveal_edge)))
                checkOnlyMarkChanged(play, wave)
                waveBounds(wave)
            }
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
                "Caption edge stability passed: light-right; 48dp; four docked edges; upright play; centered wave/play/stop; retuck identical; continuous drag across to left; no toggle; $actionMotion.\n"
            else "Caption edge stability failed at $stage: ${error.javaClass.simpleName}: ${error.message}\n")
        })
    }

    private companion object { val LIGHT: Int = Color.rgb(238, 238, 238) }
}
