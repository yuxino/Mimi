package app.yuxino.mimi.android.services.profiles

import android.content.Context
import android.util.AtomicFile
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.io.File
import java.io.FileOutputStream

/** App-private metadata and separately encrypted credential revisions. No backup/export path. */
internal class AndroidProfilePersistence(context: Context) : ProfilePersistence {
    private val catalog = AtomicFile(File(context.filesDir, "service-profiles.json"))
    private val secrets = EncryptedSharedPreferences.create(
        context, "mimi_profile_credentials", MasterKey.Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    override fun readCatalog(): String? {
        if (!catalog.baseFile.exists() && !File(catalog.baseFile.path + ".bak").exists()) return null
        return try { catalog.openRead().bufferedReader(Charsets.UTF_8).use { it.readText() } }
        catch (_: Exception) { throw IllegalStateException("profile_catalog_unavailable") }
    }

    override fun writeCatalog(value: String): Boolean {
        var output: FileOutputStream? = null
        return try {
            output = catalog.startWrite()
            output.write(value.toByteArray(Charsets.UTF_8))
            output.fd.sync()
            catalog.finishWrite(output)
            // A post-rename read failure must not call failWrite: older Android
            // implementations could delete the newly committed file after its backup is gone.
            output = null
            readCatalog() == value
        } catch (_: Exception) {
            output?.let { pending -> runCatching { catalog.failWrite(pending) } }
            false
        }
    }

    override fun readSecret(revision: String): String? = secrets.getString(revision, null)
    override fun writeSecret(revision: String, value: String): Boolean =
        secrets.edit().putString(revision, value).commit()

    override fun retainSecrets(revisions: Set<String>) {
        val editor = secrets.edit()
        secrets.all.keys.filterNot { it in revisions }.forEach(editor::remove)
        check(editor.commit()) { "profile_credential_cleanup" }
    }
}
