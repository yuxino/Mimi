package app.yuxino.mimi.android.shortcuts

import android.accessibilityservice.AccessibilityServiceInfo
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
import android.view.accessibility.AccessibilityNodeInfo
import app.yuxino.mimi.android.CaptureProbeActivity
import app.yuxino.mimi.android.MimiApplication
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.localization.InterfaceLanguage
import kotlin.math.abs
import kotlin.math.roundToInt

/** Uses real overlay windows above an ordinary clickable activity. No service or provider runs. */
internal class CaptionEdgeStabilityChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private val host get() = (context.applicationContext as MimiApplication).captionShortcuts
    private val start get() = InterfaceLanguage.context(context).getString(R.string.start_capture)
    private val reveal get() = InterfaceLanguage.context(context).getString(R.string.shortcut_reveal_edge)
    private var bounds = Rect()
    private var heldSince: Long? = null
    private var stage = "setup"
    private val touchTrace = mutableListOf<String>()
    private fun dp(value: Int) = (value * context.resources.displayMetrics.density).roundToInt()

    private fun ui(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        test.waitForIdleSync()
        failure?.let { throw it }
    }

    private fun <T : Any> await(observe: () -> T?): T {
        val deadline = SystemClock.elapsedRealtime() + 5_000
        do {
            var result: T? = null
            ui { result = observe() }
            result?.let { return it }
            SystemClock.sleep(25)
        } while (SystemClock.elapsedRealtime() < deadline)
        error("edge_timeout: $stage")
    }

    private fun character(label: String) = await { CaptionShortcutWindows.character(label, dp(48)) }
    private fun frame(view: View): Rect {
        lateinit var result: Rect
        ui { result = CaptionShortcutWindows.bounds(view) }
        return result
    }

    private fun commandBounds(right: Boolean, root: Rect): Rect {
        val command = await { CaptionShortcutWindows.command(start) }
        val result = frame(command)
        check(result.height() == dp(48) && result.width() >= dp(48) &&
            (if (right) result.centerX() < root.left else result.centerX() >= root.right) &&
            !result.contains(root.centerX(), root.centerY()) && result.centerY() in root.top until root.bottom) { "edge_command_wrong_frame: $stage, $root, $result" }
        ui {
            fun hasAccessibilityTarget(view: View): Boolean {
                if (view.importantForAccessibility == View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS) return false
                return view.isImportantForAccessibility || (view is ViewGroup &&
                    (0 until view.childCount).any { hasAccessibilityTarget(view.getChildAt(it)) })
            }
            check(!hasAccessibilityTarget(command)) { "edge_command_duplicates_accessible_button" }
        }
        return result
    }

    /** Perform the same framework accessibility action that assistive services send. */
    private fun clickAccessibleCharacter(label: String) {
        val automation = test.uiAutomation
        val originalFlags = automation.serviceInfo.flags
        automation.serviceInfo = automation.serviceInfo.apply {
            flags = originalFlags or AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        }
        try {
            // Query outside the UI thread: AccessibilityInteractionClient calls the app
            // to populate the node and dispatch the action to its accessibility delegate.
            val deadline = SystemClock.elapsedRealtime() + 5_000
            do {
                val matches = automation.windows.mapNotNull { it.root }.filter { it.packageName == context.packageName }
                    .flatMap { it.findAccessibilityNodeInfosByText(label) }
                    .filter { it.isVisibleToUser && it.isClickable && it.contentDescription == label }
                if (matches.isNotEmpty()) {
                    check(matches.size == 1 && matches.single().performAction(AccessibilityNodeInfo.ACTION_CLICK)) {
                        "edge_accessible_click_failed: $stage, matches=${matches.size}"
                    }
                    return
                }
                SystemClock.sleep(25)
            } while (SystemClock.elapsedRealtime() < deadline)
            error("edge_accessible_character_missing: $stage")
        } finally {
            automation.serviceInfo = automation.serviceInfo.apply { flags = originalFlags }
        }
    }

    private fun tucked(): View = await {
        CaptionShortcutWindows.character(reveal, dp(48))?.takeIf { CaptionShortcutWindows.command(start) == null }
    }

    private fun pixels(frame: Rect): IntArray {
        SystemClock.sleep(80) // Wait for the native compositor after the layout/focus observation.
        val screen = checkNotNull(test.uiAutomation.takeScreenshot())
        try {
            val backdropX = if (frame.centerX() > screen.width / 2) dp(8) else screen.width - dp(8)
            check(screen.getPixel(backdropX, frame.centerY()) == LIGHT) { "edge_backdrop_changed: $stage" }
            return IntArray(frame.width() * frame.height()).also {
                screen.getPixels(it, 0, frame.width(), frame.left, frame.top, frame.width(), frame.height())
            }
        } finally { screen.recycle() }
    }

    // The label may overlap the transparent padding of the 48dp hit target. Its window
    // must never cover a painted part of the character or change pixels outside the label.
    private fun checkCharacterWithCommand(idle: IntArray, root: Rect, command: Rect) {
        val expanded = pixels(root)
        for (index in idle.indices) if (idle[index] != expanded[index]) {
            val x = root.left + index % root.width()
            val y = root.top + index / root.width()
            check(idle[index] == LIGHT && command.contains(x, y)) { "edge_character_changed: $stage at ($x,$y)" }
        }
    }

    private fun characterInk(pixels: IntArray): Rect {
        val ink = Rect()
        var painted = 0
        var dark = 0
        for (i in pixels.indices) {
            val color = pixels[i]
            if (color == LIGHT) continue
            val x = i % bounds.width()
            val y = i / bounds.width()
            ink.union(x, y, x + 1, y + 1)
            painted++
            if (maxOf(Color.red(color), Color.green(color), Color.blue(color)) < 80) dark++
        }
        check(ink.height() in dp(15)..dp(32) && ink.width() in dp(7)..dp(32) && dark < painted / 3) {
            "edge_character_missing_or_dark_backplate: $stage, ink=$ink painted=$painted dark=$dark"
        }
        return ink
    }

    private fun touch(action: Int, x: Float = bounds.exactCenterX(), y: Float = bounds.exactCenterY()) {
        val now = SystemClock.uptimeMillis()
        if (action == MotionEvent.ACTION_DOWN) heldSince = now
        val event = MotionEvent.obtain(checkNotNull(heldSince), now, action, x, y, 0)
            .apply { source = InputDevice.SOURCE_TOUCHSCREEN }
        try { check(test.uiAutomation.injectInputEvent(event, true)) { "edge_touch_rejected" } }
        finally { event.recycle() }
        touchTrace.add("${MotionEvent.actionToString(action)}($x,$y)@$now")
        if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) heldSince = null
    }

    private fun tap(rect: Rect) {
        touch(MotionEvent.ACTION_DOWN, rect.exactCenterX(), rect.exactCenterY())
        touch(MotionEvent.ACTION_UP, rect.exactCenterX(), rect.exactCenterY())
    }

    private fun checkDragAcrossDisplay() {
        tap(bounds)
        val button = character(start)
        commandBounds(right = true, bounds)
        touch(MotionEvent.ACTION_DOWN)
        var fingerX = bounds.exactCenterX()
        fun moveAndHold(x: Float): Rect {
            val deadline = SystemClock.elapsedRealtime() + 2_000
            // A physical drag delivers a path, then stationary samples. Observe the actual
            // window and the absence of the companion window independently of the injected path.
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
                    hidden = CaptionShortcutWindows.command(start) == null
                    val state = "frame=$current,params=(${params.x},${params.y}),commandHidden=$hidden,pressed=${button.isPressed}"
                    if (observations.lastOrNull() != state) observations.add(state)
                }
                if (abs(current.exactCenterX() - x) <= 1 && hidden) return current
                SystemClock.sleep(20)
            } while (SystemClock.elapsedRealtime() < deadline)
            error("dragged_edge_did_not_settle_at_x=$x; observed=$observations; inputs=${touchTrace.takeLast(5)}")
        }
        stage = "held across display midpoint"
        val middle = bounds.right / 2f
        val approaching = pixels(moveAndHold(middle + 1f))
        characterInk(approaching)
        var commandDuringDrag = false
        val watch = ViewTreeObserver.OnPreDrawListener {
            if (CaptionShortcutWindows.command(start) != null) commandDuringDrag = true
            true
        }
        val dropX = bounds.width().toFloat() // Leave room for the three-second sideways snap.
        val drop: Rect
        ui { button.viewTreeObserver.addOnPreDrawListener(watch) }
        try {
            for (x in listOf(middle, middle - 1f)) {
                val crossed = pixels(moveAndHold(x))
                check(approaching.contentEquals(crossed)) { "edge_character_moved_or_flipped_at_midpoint: $x" }
            }
            stage = "held near left side"
            drop = moveAndHold(dropX)
            characterInk(pixels(drop))
        } finally { ui { button.viewTreeObserver.removeOnPreDrawListener(watch) } }
        check(!commandDuringDrag) { "edge_command_flashed_during_drag" }
        stage = "release on left"
        touch(MotionEvent.ACTION_UP, dropX)
        val released = frame(character(start))
        check(released == drop) { "edge_root_moved_on_release: $drop to $released" }
        commandBounds(right = false, released)
        stage = "tucked on left"
        val docked = frame(tucked())
        check(docked.left == 0 && docked.left < drop.left && docked.top == bounds.top) { "edge_did_not_dock_left: $docked" }
        val idle = pixels(docked)
        val ink = characterInk(idle)
        check(ink.centerX() < docked.width() / 2) { "edge_character_faces_away_from_left_side: $ink" }
        tap(docked)
        val opened = frame(character(start))
        check(opened == docked) { "left_edge_moved_on_reveal" }
        checkCharacterWithCommand(idle, opened, commandBounds(right = false, opened))
    }

    fun run() {
        var activity: CaptureProbeActivity? = null
        var window: CaptionEdgeWindow? = null
        var originalEdge: Boolean? = null
        var toggles = 0
        var backdropClicks = 0
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
                backdrop.setContentView(View(backdrop).apply {
                    setBackgroundColor(LIGHT)
                    setOnClickListener { backdropClicks++ }
                })
                val display = context.getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY)
                val windowContext = if (Build.VERSION.SDK_INT >= 30) context.createDisplayContext(checkNotNull(display))
                    .createWindowContext(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY, null) else context
                window = CaptionEdgeWindow(windowContext, CaptionEdgePlacement(1f, .5f), { toggles++ }, {}).also { it.show() }
            }
            stage = "right tucked"
            bounds = frame(tucked())
            check(bounds.right == context.resources.displayMetrics.widthPixels)
            val idle = pixels(bounds)
            val ink = characterInk(idle)
            check(ink.centerX() > bounds.width() / 2) { "edge_character_faces_away_from_right_side: $ink" }
            SystemClock.sleep(270)
            check(idle.contentEquals(pixels(bounds))) { "idle_edge_pulsed" }
            stage = "first physical tap"
            tap(bounds)
            val button = character(start)
            val command = commandBounds(right = true, bounds)
            ui { check(button.createAccessibilityNodeInfo().isClickable) { "edge_has_no_accessible_action" } }
            check(frame(button) == bounds && toggles == 0) { "edge_character_moved_or_started_on_reveal" }
            checkCharacterWithCommand(idle, bounds, command)
            stage = "long press"
            touch(MotionEvent.ACTION_DOWN)
            SystemClock.sleep(ViewConfiguration.getLongPressTimeout().toLong() + 100)
            checkCharacterWithCommand(idle, bounds, command)
            touch(MotionEvent.ACTION_UP)
            val released = SystemClock.elapsedRealtime()
            checkCharacterWithCommand(idle, bounds, command)
            check(toggles == 0) { "long_press_activated_edge" }
            stage = "three-second retuck"
            check(frame(tucked()) == bounds && pixels(bounds).contentEquals(idle)) { "retuck_changed_character" }
            check(SystemClock.elapsedRealtime() - released in 2_500L..5_000L) { "edge_retuck_timing" }
            stage = "retired command lets a physical tap through"
            tap(command)
            await { Unit.takeIf { backdropClicks == 1 && toggles == 0 } }
            stage = "physical tap on command"
            tap(bounds)
            tap(commandBounds(right = true, bounds))
            await { Unit.takeIf { toggles == 1 && backdropClicks == 1 } }
            tucked()
            stage = "second physical tap on character keeps the command open"
            tap(bounds)
            character(start)
            SystemClock.sleep(1_650)
            tap(bounds)
            SystemClock.sleep(1_650)
            commandBounds(right = true, bounds)
            check(toggles == 1 && backdropClicks == 1) { "repeated_character_tap_activated_a_command" }
            tucked()
            stage = "accessibility click on the tucked character"
            clickAccessibleCharacter(reveal)
            character(start)
            check(toggles == 1) { "tucked_accessible_click_started_captions" }
            stage = "accessibility click on the available action"
            clickAccessibleCharacter(start)
            await { Unit.takeIf { toggles == 2 } }
            tucked()
            stage = "drag across display"
            checkDragAcrossDisplay()
            check(toggles == 2 && backdropClicks == 1) { "drag_fired_a_click" }
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
                "Caption edge stability passed: 48dp; fixed character; independent start command; tap-through after retuck; " +
                    "single accessible action; cross-screen drag; animations enabled=${ValueAnimator.areAnimatorsEnabled()}.\n"
            else "Caption edge stability failed at $stage: ${error.javaClass.simpleName}: ${error.message}\n")
        })
    }

    private companion object { val LIGHT: Int = Color.rgb(238, 238, 238) }
}
