package app.yuxino.mimi.android

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.Executor

class AppUpdateCheckTest {
    private class QueuedExecutor : Executor {
        lateinit var task: Runnable
        override fun execute(command: Runnable) { task = command }
    }

    private fun release(version: String = "1.6.17"): JSONObject {
        val base = "https://github.com/yuxino/Mimi/releases"
        val name = "mimi_${version}_android.apk"
        return JSONObject().put("draft", false).put("prerelease", false).put("tag_name", "v$version")
            .put("html_url", "$base/tag/v$version").put("assets", JSONArray().put(JSONObject()
                .put("name", name).put("state", "uploaded").put("size", 1)
                .put("browser_download_url", "$base/download/v$version/$name")))
    }

    private fun result(release: JSONObject, installed: String = "1.6.16") =
        parseAppUpdate(release.toString(), installed)

    @Test fun comparesNumericVersionsWithoutOfferingDowngrades() {
        val update = result(release("1.10.0"), "1.9.9") as AppUpdateResult.Available
        assertEquals("1.10.0", update.release.version.toString())
        assertEquals("https://github.com/yuxino/Mimi/releases/download/v1.10.0/mimi_1.10.0_android.apk", update.release.downloadUrl)
        assertEquals(AppUpdateResult.UpToDate, result(release(), "1.6.17"))
        assertEquals(AppUpdateResult.UpToDate, result(release(), "2.0.0"))
    }

    @Test fun neverTreatsPreviewOrMalformedVersionsAsStableUpdates() {
        for (version in listOf("1.6", "1.6.17.0", "1.6.17-rc.1", "1.6.17+build", "01.6.17", "1.6.-1", "999999999999.1.1", " 1.6.17")) {
            assertEquals(version, AppUpdateResult.Failed, result(release(version)))
        }
        assertEquals(AppUpdateResult.Failed, result(release().put("tag_name", "1.6.17")))
        assertEquals(AppUpdateResult.Failed, result(release(), "1.6.16-dev"))
        assertEquals(AppUpdateResult.Failed, result(release(), "unknown"))
        assertEquals(AppUpdateResult.Failed, result(release().put("draft", true)))
        assertEquals(AppUpdateResult.Failed, result(release().put("prerelease", true)))
        assertEquals(AppUpdateResult.Failed, result(release().put("draft", "false")))
        assertEquals(AppUpdateResult.Failed, result(release().apply { remove("prerelease") }))
        assertEquals(AppUpdateResult.Failed, parseAppUpdate("not json", "1.6.16"))
    }

    @Test fun requiresOneUploadedAndroidAssetBeforeClaimingAnUpdateOrCurrentVersion() {
        assertEquals(AppUpdateResult.Failed, result(release().put("assets", JSONArray())))
        assertEquals(AppUpdateResult.Failed, result(release().put("assets", JSONArray()), "1.6.17"))
        val wrongAsset = release().apply { getJSONArray("assets").getJSONObject(0).put("name", "mimi_1.6.17_android.apk.exe") }
        assertEquals(AppUpdateResult.Failed, result(wrongAsset))
        for ((field, value) in listOf("size" to 0, "size" to -1, "state" to "new")) {
            assertEquals(AppUpdateResult.Failed, result(release().apply { getJSONArray("assets").getJSONObject(0).put(field, value) }))
        }
        assertEquals(AppUpdateResult.Failed, result(release().apply {
            getJSONArray("assets").put(JSONObject(getJSONArray("assets").getJSONObject(0).toString()))
        }))
    }

    @Test fun rejectsExternalOrMismatchedDownloadAndReleaseDestinations() {
        val official = "https://github.com/yuxino/Mimi/releases/download/v1.6.17/mimi_1.6.17_android.apk"
        for (url in listOf(
            official.replace("https:", "http:"), official.replace("github.com/", "github.com.evil.example/"),
            official.replace("yuxino/Mimi", "another/Mimi"), official.replace("v1.6.17/", "v1.6.16/"),
            official.replace("github.com/", "github.com@evil.example/"), "$official?redirect=elsewhere",
            "$official#fragment",
        )) {
            assertEquals(url, AppUpdateResult.Failed, result(release().apply {
                getJSONArray("assets").getJSONObject(0).put("browser_download_url", url)
            }))
        }
        assertEquals(AppUpdateResult.Failed, result(release().put("html_url", "https://github.com/another/Mimi/releases/tag/v1.6.17")))
    }

    @Test fun completedNativeRequestReturnsTheVersionAndReleasesItsTicket() {
        val operations = mutableListOf<String>()
        val checker = GitHubAppUpdateChecker(exchange = { request ->
            val operation = request.getString("operation")
            operations += operation
            when (operation) {
                "app_update_start" -> JSONObject().put("handle", 42)
                "app_update_poll" -> {
                    assertEquals(42, request.getLong("handle"))
                    JSONObject().put("finished", true).put("response", release().toString())
                }
                "app_update_cancel" -> { assertEquals(42, request.getLong("handle")); JSONObject() }
                else -> error("unexpected operation")
            }
        }, networkSnapshot = { JSONObject() }, executor = Executor { it.run() })
        val results = mutableListOf<AppUpdateResult>()
        checker.check("1.6.16", results::add)
        assertTrue(results.single() is AppUpdateResult.Available)
        assertEquals(listOf("app_update_start", "app_update_poll", "app_update_cancel"), operations)
    }

    @Test fun cancelledQueuedRequestDoesNotReadTrustOrCrossJni() {
        val executor = QueuedExecutor()
        val checker = GitHubAppUpdateChecker(exchange = { error("cancelled request crossed JNI") },
            networkSnapshot = { error("cancelled request read trust") }, executor = executor)
        val call = checker.check("1.6.17") { fail("cancelled request delivered") }
        call.cancel()
        executor.task.run()
    }

    @Test fun cancellationDuringNativeCreationReleasesTheTicketWithoutPolling() {
        val executor = QueuedExecutor()
        val operations = mutableListOf<String>()
        lateinit var call: AppUpdateCall
        val checker = GitHubAppUpdateChecker(exchange = { request ->
            val operation = request.getString("operation")
            operations += operation
            when (operation) {
                "app_update_start" -> { call.cancel(); JSONObject().put("handle", 43) }
                "app_update_cancel" -> { assertEquals(43, request.getLong("handle")); JSONObject() }
                else -> error("cancelled creation was polled")
            }
        }, networkSnapshot = { JSONObject() }, executor = executor)
        call = checker.check("1.6.17") { fail("cancelled creation delivered") }
        executor.task.run()
        assertEquals(listOf("app_update_start", "app_update_cancel"), operations)
    }

    @Test fun cancellationDuringPollCannotDeliverALateNativeResponse() {
        val executor = QueuedExecutor()
        lateinit var call: AppUpdateCall
        val checker = GitHubAppUpdateChecker(exchange = { request ->
            when (request.getString("operation")) {
                "app_update_start" -> JSONObject().put("handle", 44)
                "app_update_poll" -> { call.cancel(); JSONObject().put("finished", true).put("response", release().toString()) }
                "app_update_cancel" -> JSONObject()
                else -> error("unexpected operation")
            }
        }, networkSnapshot = { JSONObject() }, executor = executor)
        call = checker.check("1.6.16") { fail("cancelled poll delivered") }
        executor.task.run()
    }
}
