package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.app.LocaleManager
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.os.LocaleList
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.os.LocaleListCompat
import app.yuxino.mimi.android.captions.CaptionDisplayMode
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SharedSubtitleCore
import app.yuxino.mimi.android.provider.SubtitleBus
import java.io.File
import org.json.JSONObject

/** Synthetic shared snapshots in the real native overlay; no capture or provider. */
internal class RuntimeFeedbackChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
    fun run(arguments: Bundle?) {
        var stage = "preconditions"
        var failure: String? = null
        val previousLocales = AppCompatDelegate.getApplicationLocales()
        val localeManager = if (Build.VERSION.SDK_INT >= 33) context.getSystemService(LocaleManager::class.java) else null
        val previousSystemLocales = if (Build.VERSION.SDK_INT >= 33) localeManager?.applicationLocales else null
        val locale = arguments?.getString("locale") ?: "en"
        val previousImmersive = SettingsStore.immersiveSubtitles(context)
        val previousMode = SettingsStore.displayMode(context)
        try {
            check(!MimiService.isRunning && !SettingsStore.isConfigured(context))
            onUi {
                if (Build.VERSION.SDK_INT >= 33) localeManager?.applicationLocales = LocaleList.forLanguageTags(locale)
                else AppCompatDelegate.setApplicationLocales(LocaleListCompat.forLanguageTags(locale))
                SettingsStore.setImmersiveSubtitles(context, false)
                SettingsStore.setDisplayMode(context, CaptionDisplayMode.BILINGUAL)
            }
            context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW).putExtra("preview_over_own_ui", true))
            await { root() != null }
            val created = SharedSubtitleCore.exchange(JSONObject().put("operation", JSONObject().put("type", "create").put("history_limit", 0)))
            val response = SharedSubtitleCore.exchange(JSONObject().put("state", created.getString("state"))
                .put("operation", JSONObject().put("type", "apply").put("event", JSONObject()
                    .put("type", "final_pair").put("source", "Synthetic status sample.").put("translation", "合成状态示例。"))))
            val states = listOf(
                "connecting" to JSONObject().put("connectionStatus", "connecting"),
                "rate-retry" to recovery("rateLimited", true),
                "rate-wait" to recovery("rateLimited", false),
                "unavailable-retry" to recovery("temporarilyUnavailable", true),
                "unavailable-wait" to recovery("temporarilyUnavailable", false),
                "delayed" to JSONObject().put("connectionStatus", "listening").put("isTranslationTimedOut", true),
            )
            fun apply(fields: JSONObject) {
                onUi { SubtitleBus.onRuntimeSnapshot(fields.put("atomicPreview", true)
                    .put("snapshot", response.getJSONObject("snapshot")), false) }
                test.waitForIdleSync()
            }
            for (immersive in listOf(false, true)) {
                val mode = if (immersive) "immersive" else "normal"
                stage = "$locale-$mode-visible"
                onUi { SettingsStore.setImmersiveSubtitles(context, immersive) }
                await { root()?.findViewWithTag<View>(if (immersive) "compact-subtitle" else "expanded-subtitles")?.isShown == true }
                for ((name, fields) in states) {
                    stage = "$locale-$mode-$name"
                    apply(JSONObject(fields.toString()))
                    val resource = checkNotNull(runtimeFeedbackResource(SubtitleBus.runtimeFeedback))
                    await { texts(root()!!).any { it.isShown && it.text.toString() == InterfaceLanguage.context(context).getString(resource) } }
                    onUi {
                        val expected = InterfaceLanguage.context(context).getString(resource)
                        val text = texts(root()!!).first { it.isShown && it.text.toString() == expected }
                        val visible = Rect()
                        check(text.getGlobalVisibleRect(visible) && visible.height() >= text.height)
                        check(text.layout.height <= text.height - text.paddingTop - text.paddingBottom)
                        check(SubtitleBus.displayTranslation == "合成状态示例。")
                    }
                    capture("$locale-$mode-$name")
                }
                apply(JSONObject().put("connectionStatus", "listening"))
                await { texts(root()!!).none { it.isShown && it.text.toString() == InterfaceLanguage.context(context).getString(R.string.runtime_translation_delayed) } }
                onUi {
                    check(runtimeFeedbackResource(SubtitleBus.runtimeFeedback) == null)
                    check(texts(root()!!).none { it.isShown && it.text.toString() == InterfaceLanguage.context(context).getString(R.string.runtime_translation_delayed) })
                }
            }
            check(!MimiService.isRunning)
        } catch (error: Throwable) {
            val location = error.stackTrace.firstOrNull { it.fileName == "RuntimeFeedbackChecks.kt" }
            failure = "$stage:${error.javaClass.simpleName}:${location?.lineNumber ?: 0}"
        }
        finally {
            context.startService(MimiService.stopIntent(context))
            test.waitForIdleSync()
            onUi {
                SettingsStore.setImmersiveSubtitles(context, previousImmersive)
                SettingsStore.setDisplayMode(context, previousMode)
                if (Build.VERSION.SDK_INT >= 33 && previousSystemLocales != null) localeManager?.applicationLocales = previousSystemLocales
                else AppCompatDelegate.setApplicationLocales(previousLocales)
            }
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "Runtime feedback passed=${failure == null}; locale=$locale; failureStage=${failure ?: "none"}; native normal/immersive; synthetic snapshots only.\n")
        })
    }
    private fun recovery(reason: String, scheduled: Boolean) = JSONObject().put("connectionStatus", "listening")
        .put("translationRecovery", JSONObject().put("reason", reason).put("retryAfterMs", if (scheduled) 4000 else 0).put("retryScheduled", scheduled))
    private fun root(): View? = WindowInspector.getGlobalWindowViews().firstOrNull { it.tag == "mimi-overlay" }
    private fun texts(view: View): List<TextView> = (if (view is TextView) listOf(view) else emptyList()) +
        if (view is ViewGroup) (0 until view.childCount).flatMap { texts(view.getChildAt(it)) } else emptyList()
    private fun await(predicate: () -> Boolean) {
        val end = SystemClock.elapsedRealtime() + 5000
        while (SystemClock.elapsedRealtime() < end) {
            var ready = false; onUi { ready = predicate() }
            if (ready) return
            SystemClock.sleep(50)
        }
        error("native_overlay_timeout")
    }
    private fun capture(name: String) {
        test.waitForIdleSync()
        SystemClock.sleep(120) // Allow the native compositor to present the updated text.
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        val dir = checkNotNull(context.getExternalFilesDir("runtime-feedback"))
        File(dir, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
    }
}
