package app.yuxino.mimi.android

import android.app.Activity
import android.app.Application
import android.content.res.Configuration
import android.os.Bundle
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.services.editor.ServiceSettingsActivity
import app.yuxino.mimi.android.sessions.SavedSessionsActivity
import app.yuxino.mimi.android.shortcuts.CaptionShortcutHost

/** Captions must not cover Mimi's own controls; capture continues in the background. */
class MimiApplication : Application() {
    internal lateinit var captionShortcuts: CaptionShortcutHost
        private set

    override fun onCreate() {
        super.onCreate()
        captionShortcuts = CaptionShortcutHost(this)
        registerActivityLifecycleCallbacks(object : ActivityLifecycleCallbacks {
            override fun onActivityStarted(activity: Activity) {
                if (activity is MainActivity || activity is ServiceSettingsActivity || activity is SavedSessionsActivity)
                    setOwnUiVisible(activity, true)
            }
            override fun onActivityStopped(activity: Activity) = setOwnUiVisible(activity, false)
            override fun onActivityCreated(activity: Activity, state: Bundle?) = Unit
            override fun onActivityResumed(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = setOwnUiVisible(activity, false)
        })
    }

    private fun setOwnUiVisible(activity: Activity, visible: Boolean) {
        MimiService.setOwnUiVisible(activity, visible)
        captionShortcuts.setOwnUiVisible(MimiService.ownUiVisible)
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        captionShortcuts.refresh()
    }
}
