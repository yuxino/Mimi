package app.yuxino.mimi.android

import app.yuxino.mimi.android.provider.NativeRuntimeConfiguration
import org.json.JSONObject

/** Resolve through the PC factory; never display an ignored legacy model override. */
internal fun runtimeModelNames(configuration: JSONObject): List<String> {
    val names = NativeRuntimeConfiguration.endpoints(configuration).getJSONArray("modelNames")
    return (0 until names.length()).map(names::getString)
}

internal fun modelNamesLabel(names: List<String>): String = names.joinToString(" · ") {
    when (it) {
        "qwen-audio-3.0-asr-flash-streaming" -> "Audio 3.0"
        "qwen-mt-lite" -> "Qwen-MT Lite"
        "qwen-mt-flash" -> "Qwen-MT Flash"
        "qwen-mt-plus" -> "Qwen-MT Plus"
        else -> it
    }
}
