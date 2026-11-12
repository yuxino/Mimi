package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.content.res.Configuration
import android.os.Bundle
import android.os.LocaleList
import android.view.ContextThemeWrapper
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout

/** Repeated native layout, including transitions, without an activity, capture or provider. */
internal class LayoutWorkloadChecks(private val test: Instrumentation) {
    fun run() {
        var replacements = 0
        var cases = 0
        val geometry = mutableListOf<Int>()
        val result = runCatching {
            ui {
                for (locale in listOf("en", "de", "ja")) for (fontScale in listOf(1f, 1.5f)) {
                    val config = Configuration(test.targetContext.resources.configuration).apply {
                        setLocales(LocaleList.forLanguageTags(locale))
                        this.fontScale = fontScale
                    }
                    val context = ContextThemeWrapper(test.targetContext.createConfigurationContext(config), R.style.Theme_Mimi)
                    val root = LayoutInflater.from(context).inflate(R.layout.activity_main, null) as ViewGroup
                    val workspace = root.findViewById<CaptionWorkspace>(R.id.caption_workspace)
                    val footer = (0 until root.childCount).map(root::getChildAt).filterIsInstance<SessionControlsLayout>().single()
                    val watched = listOf(workspace.getChildAt(0), workspace.getChildAt(1), footer.getChildAt(0), footer.getChildAt(1))
                    val density = context.resources.displayMetrics.density
                    // Return to portrait after landscape/hidden states to catch stale weights and margins.
                    for ((widthDp, heightDp, visible) in listOf(Triple(360, 800, true), Triple(840, 400, true),
                        Triple(840, 400, false), Triple(360, 800, true), Triple(280, 640, true))) {
                        workspace.showsPreview = visible
                        val width = (widthDp * density).toInt()
                        val height = (heightDp * density).toInt()
                        fun layout() {
                            forceLayout(root)
                            root.measure(View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
                                View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY))
                            root.layout(0, 0, width, height)
                        }
                        layout()
                        val expected = bounds(watched)
                        geometry.addAll(expected)
                        check(workspace.orientation == if (visible && widthDp > heightDp) LinearLayout.HORIZONTAL else LinearLayout.VERTICAL)
                        check(workspace.getChildAt(0).visibility == if (visible) View.VISIBLE else View.GONE)
                        check(footer.getChildAt(1).width > 0 && footer.getChildAt(1).height > 0)
                        if (footer.orientation == LinearLayout.HORIZONTAL) {
                            check(footer.getChildAt(0).right <= footer.getChildAt(1).left)
                        } else {
                            check(footer.getChildAt(0).bottom <= footer.getChildAt(1).top)
                        }
                        repeat(50) {
                            val previous = watched.map { it.layoutParams }
                            layout()
                            replacements += watched.indices.count { previous[it] !== watched[it].layoutParams }
                            check(bounds(watched) == expected) { "Repeated layout changed visible geometry" }
                        }
                        cases++
                    }
                }
            }
            check(replacements == 0) { "Stable layout replaced $replacements child parameter objects" }
        }
        test.finish(if (result.isSuccess) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "Layout workload: cases=$cases passes=${cases * 50} parameterReplacements=$replacements geometry=${geometry.hashCode()}\n" +
                result.fold({ "PASS: stable native geometry and reusable child parameters.\n" }, { "FAIL: ${it.message}\n" }))
        })
    }

    private fun forceLayout(view: View) {
        view.forceLayout()
        if (view is ViewGroup) for (index in 0 until view.childCount) forceLayout(view.getChildAt(index))
    }

    private fun ui(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }

    private fun bounds(views: List<View>) = views.flatMap { listOf(it.left, it.top, it.right, it.bottom) }
}
