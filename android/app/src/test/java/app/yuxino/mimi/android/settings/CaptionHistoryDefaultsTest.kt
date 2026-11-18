package app.yuxino.mimi.android.settings

import java.nio.file.Files
import org.junit.Assert.*
import org.junit.Test

class CaptionHistoryDefaultsTest {
    @Test fun onlyFreshInstallsReceiveTheNewDefault() {
        val root = Files.createTempDirectory("caption-defaults").toFile()
        try {
            val files = java.io.File(root, "files").apply { mkdirs() }
            assertTrue(isFreshCaptionInstall(root, files, false))
            assertFalse(isFreshCaptionInstall(root, files, true))
            for (path in listOf("shared_prefs/mimi_secure.xml", "shared_prefs/mimi_secure.xml.bak",
                "files/service-profiles.json", "files/service-profiles.json.bak")) {
                val previous = java.io.File(root, path)
                previous.parentFile!!.mkdirs()
                // Empty stores also belong to old users who never changed the zero default.
                previous.writeText("")
                assertFalse(path, isFreshCaptionInstall(root, files, false))
                assertTrue(previous.delete())
            }
        } finally { root.deleteRecursively() }
    }
}
