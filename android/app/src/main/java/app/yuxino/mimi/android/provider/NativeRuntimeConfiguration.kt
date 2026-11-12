package app.yuxino.mimi.android.provider

import org.json.JSONObject

/** JNI binding to the actual provider runtime used by the desktop application. */
internal object NativeRuntimeConfiguration {
    init { System.loadLibrary("mimi_android_jni") }
    @JvmStatic external fun exchangeRaw(request: String): String
    @JvmStatic external fun pcmRaw(handle: Long, pcm: ByteArray): Boolean
    fun exchange(request: JSONObject): JSONObject = JSONObject(exchangeRaw(request.toString()))
    fun capabilities(provider: String, route: String, target: String, alibabaRealtimeMode: String = "asrMt", xaiSpeechMode: String = "voice", speechTranslationMode: String = "simultaneous"): JSONObject = exchange(
        JSONObject().put("operation", "capabilities").put("provider", provider).put("route", route).put("target", target)
            .put("alibabaRealtimeMode", wireMode(provider, alibabaRealtimeMode)).put("xaiSpeechMode", xaiSpeechMode).put("speechTranslationMode", speechTranslationMode))
    fun normalize(provider: String, route: String, source: String, target: String, alibabaRealtimeMode: String = "asrMt", xaiSpeechMode: String = "voice", speechTranslationMode: String = "simultaneous"): JSONObject = exchange(
        JSONObject().put("operation", "normalize").put("provider", provider).put("route", route).put("source", source).put("target", target)
            .put("alibabaRealtimeMode", wireMode(provider, alibabaRealtimeMode)).put("xaiSpeechMode", xaiSpeechMode).put("speechTranslationMode", speechTranslationMode))
    private fun wireMode(provider: String, mode: String): String =
        if (provider == "alibabaCloud" && mode == "liveTranslate") "liveTranslate" else "disabled"
    fun endpoints(configuration: JSONObject): JSONObject = exchange(
        JSONObject().put("operation", "endpoints").put("configuration", configuration))
    fun textEndpointsEquivalent(provider: TextTranslationProvider, saved: String, draft: String): Boolean = exchange(
        JSONObject().put("operation", "text_endpoints_equivalent").put("route", provider.runtimeRoute())
            .put("savedEndpoint", saved).put("draftEndpoint", draft)).getBoolean("equivalent")
    fun textEndpoint(provider: TextTranslationProvider, input: String, allowLocalHttp: Boolean): String = exchange(
        JSONObject().put("operation", "normalize_text_endpoint").put("route", provider.runtimeRoute())
            .put("input", input).put("allowLocalHttp", allowLocalHttp)).getString("endpoint")
    fun resolveCredentialsDraft(provider: ServiceProvider, mode: String, draft: JSONObject, saved: JSONObject?): JSONObject = exchange(
        JSONObject().put("operation", "resolve_credentials_draft").put("provider", provider.wireProvider)
            .put("speechTranslationMode", mode).put("draft", draft).put("saved", saved ?: JSONObject.NULL))
        .getJSONObject("credentials")
    val policy: JSONObject by lazy { exchange(JSONObject().put("operation", "policy")) }
    fun command(operation: String, handle: Long): JSONObject = exchange(JSONObject().put("operation", operation).put("handle", handle))
    fun poll(handle: Long, sinceVersion: Long? = null): JSONObject = exchange(runtimePollRequest(handle, sinceVersion))
}

internal fun runtimePollRequest(handle: Long, sinceVersion: Long? = null): JSONObject {
    require(sinceVersion == null || sinceVersion >= 0) { "native_runtime_invalid_request" }
    return JSONObject().put("operation", "poll").put("handle", handle).apply {
        if (sinceVersion != null) put("sinceVersion", sinceVersion)
    }
}
