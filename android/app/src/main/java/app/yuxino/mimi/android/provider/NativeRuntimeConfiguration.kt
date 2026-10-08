package app.yuxino.mimi.android.provider

import org.json.JSONObject

/** JNI binding to the actual provider runtime used by the desktop application. */
internal object NativeRuntimeConfiguration {
    init { System.loadLibrary("mimi_android_jni") }
    @JvmStatic external fun exchangeRaw(request: String): String
    @JvmStatic external fun pcmRaw(handle: Long, pcm: ByteArray): Boolean
    fun exchange(request: JSONObject): JSONObject = JSONObject(exchangeRaw(request.toString()))
    fun capabilities(provider: String, route: String, target: String): JSONObject = exchange(
        JSONObject().put("operation", "capabilities").put("provider", provider).put("route", route).put("target", target))
    fun normalize(provider: String, route: String, source: String, target: String): JSONObject = exchange(
        JSONObject().put("operation", "normalize").put("provider", provider).put("route", route).put("source", source).put("target", target))
    fun endpoints(configuration: JSONObject): JSONObject = exchange(
        JSONObject().put("operation", "endpoints").put("configuration", configuration))
    val policy: JSONObject by lazy { exchange(JSONObject().put("operation", "policy")) }
    fun command(operation: String, handle: Long): JSONObject = exchange(JSONObject().put("operation", operation).put("handle", handle))
}
