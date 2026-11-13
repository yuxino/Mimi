package app.yuxino.mimi.android

import org.json.JSONObject
import app.yuxino.mimi.android.provider.NativeRuntimeConfiguration
import app.yuxino.mimi.android.provider.NativeRuntimeNetwork
import java.util.concurrent.Executor
import java.util.concurrent.Executors

internal data class AppVersion private constructor(val major: Int, val minor: Int, val patch: Int) : Comparable<AppVersion> {
    override fun compareTo(other: AppVersion): Int =
        compareValuesBy(this, other, AppVersion::major, AppVersion::minor, AppVersion::patch)

    override fun toString(): String = "$major.$minor.$patch"

    companion object {
        fun parse(value: String): AppVersion? {
            if (!Regex("(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)").matches(value)) return null
            val parts = value.split('.').map { it.toIntOrNull() ?: return null }
            return AppVersion(parts[0], parts[1], parts[2])
        }
    }
}

internal data class AppRelease(val version: AppVersion, val downloadUrl: String)

internal sealed interface AppUpdateResult {
    data object UpToDate : AppUpdateResult
    data class Available(val release: AppRelease) : AppUpdateResult
    data object Failed : AppUpdateResult
}

/** Only the published, universal Android artifact can become an update action. */
internal fun parseAppUpdate(response: String, installedVersion: String): AppUpdateResult {
    val installed = AppVersion.parse(installedVersion) ?: return AppUpdateResult.Failed
    return try {
        val release = JSONObject(response)
        require(release.get("draft") == false && release.get("prerelease") == false)
        val tag = release.getString("tag_name")
        require(tag.startsWith('v'))
        val version = requireNotNull(AppVersion.parse(tag.removePrefix("v")))
        val base = "https://github.com/yuxino/Mimi/releases"
        require(release.getString("html_url") == "$base/tag/$tag")
        val name = "mimi_${version}_android.apk"
        val url = "$base/download/$tag/$name"
        val assets = release.getJSONArray("assets")
        val matches = (0 until assets.length()).mapNotNull { assets.optJSONObject(it) }
            .filter { it.optString("name") == name }
        val artifact = matches.single()
        require(artifact.getString("state") == "uploaded" && artifact.getLong("size") > 0)
        require(artifact.getString("browser_download_url") == url)
        if (version > installed) AppUpdateResult.Available(AppRelease(version, url))
        else AppUpdateResult.UpToDate
    } catch (_: Exception) {
        AppUpdateResult.Failed
    }
}

internal fun interface AppUpdateCall { fun cancel() }
internal fun interface AppUpdateChecker {
    fun check(installedVersion: String, completed: (AppUpdateResult) -> Unit): AppUpdateCall
}

/** UI adapter only: Rust owns the fixed HTTP request, limits, TLS and cancellation. */
internal class GitHubAppUpdateChecker(
    private val exchange: (JSONObject) -> JSONObject = NativeRuntimeConfiguration::exchange,
    private val networkSnapshot: () -> JSONObject = NativeRuntimeNetwork::updateSnapshot,
    private val executor: Executor = worker,
) : AppUpdateChecker {
    override fun check(installedVersion: String, completed: (AppUpdateResult) -> Unit): AppUpdateCall {
        val lock = Any()
        var cancelled = false
        var handle: Long? = null
        fun cancelHandle(value: Long) {
            runCatching { exchange(JSONObject().put("operation", "app_update_cancel").put("handle", value)) }
        }
        fun deliver(result: AppUpdateResult) = synchronized(lock) { if (!cancelled) completed(result) }
        executor.execute {
            var ownedHandle: Long? = null
            try {
                if (synchronized(lock) { cancelled }) return@execute
                require(AppVersion.parse(installedVersion) != null)
                val network = networkSnapshot()
                if (synchronized(lock) { cancelled }) return@execute
                val created = exchange(JSONObject().put("operation", "app_update_start").put("network", network)).getLong("handle")
                ownedHandle = created
                synchronized(lock) { handle = created }
                while (!synchronized(lock) { cancelled }) {
                    val state = exchange(JSONObject().put("operation", "app_update_poll").put("handle", created))
                    if (state.getBoolean("finished")) {
                        deliver(if (state.isNull("response")) AppUpdateResult.Failed
                            else parseAppUpdate(state.getString("response"), installedVersion))
                        break
                    }
                    Thread.sleep(100)
                }
            } catch (_: Exception) {
                deliver(AppUpdateResult.Failed)
            } finally {
                synchronized(lock) { handle = null }
                ownedHandle?.let(::cancelHandle)
            }
        }
        return AppUpdateCall {
            val current = synchronized(lock) { cancelled = true; handle }
            current?.let(::cancelHandle)
        }
    }

    private companion object {
        val worker: Executor = Executors.newSingleThreadExecutor { action ->
            Thread(action, "mimi-update-check-ui").apply { isDaemon = true }
        }
    }
}
