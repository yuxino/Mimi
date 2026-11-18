package app.yuxino.mimi.android.settings

import java.io.File

/** Check before creating preferences; even an empty old store preserves the old default. */
internal fun isFreshCaptionInstall(dataDir: File, filesDir: File, hasSeenSetup: Boolean): Boolean =
    !hasSeenSetup && listOf(
        File(dataDir, "shared_prefs/mimi_secure.xml"),
        File(dataDir, "shared_prefs/mimi_secure.xml.bak"),
        File(filesDir, "service-profiles.json"),
        File(filesDir, "service-profiles.json.bak"),
    ).none { it.exists() }
