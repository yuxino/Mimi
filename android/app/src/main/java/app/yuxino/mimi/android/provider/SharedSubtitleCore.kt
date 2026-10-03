package app.yuxino.mimi.android.provider

import org.json.JSONObject

/** Stateless JSON transport. The caller owns every serialized reducer state. */
internal object SharedSubtitleCore {
    init {
        // Missing or incompatible native code is a build/runtime error. There
        // is deliberately no Kotlin reducer or silent loading fallback.
        System.loadLibrary("mimi_android_jni")
    }

    @JvmStatic
    external fun exchangeRaw(request: String): String

    fun exchange(request: JSONObject): JSONObject = JSONObject(exchangeRaw(request.toString()))

    val policy: JSONObject by lazy {
        exchange(JSONObject().put("operation", JSONObject().put("type", "policy"))).getJSONObject("policy")
    }
}
