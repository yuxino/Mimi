package app.yuxino.mimi.android

import android.app.Activity
import android.app.Application
import android.os.Bundle
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.services.editor.ServiceSettingsActivity
import app.yuxino.mimi.android.sessions.SavedSessionsActivity

/** Captions must not cover Mimi's own controls; capture continues in the background. */
class MimiApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        registerActivityLifecycleCallbacks(object : ActivityLifecycleCallbacks {
            override fun onActivityStarted(activity: Activity) {
                if (activity is MainActivity || activity is ServiceSettingsActivity || activity is SavedSessionsActivity)
                    MimiService.setOwnUiVisible(activity, true)
            }
            override fun onActivityStopped(activity: Activity) = MimiService.setOwnUiVisible(activity, false)
            override fun onActivityCreated(activity: Activity, state: Bundle?) = Unit
            override fun onActivityResumed(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = MimiService.setOwnUiVisible(activity, false)
        })
    }
}
