package app.yuxino.mimi.android.settings

import android.content.SharedPreferences
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class SettingsStoreRollbackTest {
    private class FailingCommitPreferences(initial: Map<String, Any?> = emptyMap()) : SharedPreferences {
        private val map = initial.toMutableMap()
        var failCommit = false

        override fun getAll(): MutableMap<String, *> = map.toMutableMap()
        override fun getString(key: String?, defValue: String?): String? = (map[key] as? String) ?: defValue
        override fun getStringSet(key: String?, defValues: MutableSet<String>?): MutableSet<String>? {
            @Suppress("UNCHECKED_CAST")
            return (map[key] as? Set<String>)?.toMutableSet() ?: defValues
        }
        override fun getInt(key: String?, defValue: Int): Int = (map[key] as? Int) ?: defValue
        override fun getLong(key: String?, defValue: Long): Long = (map[key] as? Long) ?: defValue
        override fun getFloat(key: String?, defValue: Float): Float = (map[key] as? Float) ?: defValue
        override fun getBoolean(key: String?, defValue: Boolean): Boolean = (map[key] as? Boolean) ?: defValue
        override fun contains(key: String?): Boolean = map.containsKey(key)

        override fun edit(): SharedPreferences.Editor = object : SharedPreferences.Editor {
            private val pending = mutableMapOf<String, Any?>()
            private val removals = mutableSetOf<String>()
            private var clearAll = false

            override fun putString(key: String, value: String?): SharedPreferences.Editor = apply {
                if (value == null) removals.add(key) else { removals.remove(key); pending[key] = value }
            }
            override fun putStringSet(key: String, values: MutableSet<String>?): SharedPreferences.Editor = apply {
                if (values == null) removals.add(key) else { removals.remove(key); pending[key] = values.toSet() }
            }
            override fun putInt(key: String, value: Int): SharedPreferences.Editor = apply {
                removals.remove(key); pending[key] = value
            }
            override fun putLong(key: String, value: Long): SharedPreferences.Editor = apply {
                removals.remove(key); pending[key] = value
            }
            override fun putFloat(key: String, value: Float): SharedPreferences.Editor = apply {
                removals.remove(key); pending[key] = value
            }
            override fun putBoolean(key: String, value: Boolean): SharedPreferences.Editor = apply {
                removals.remove(key); pending[key] = value
            }
            override fun remove(key: String): SharedPreferences.Editor = apply {
                pending.remove(key); removals.add(key)
            }
            override fun clear(): SharedPreferences.Editor = apply { clearAll = true }

            override fun commit(): Boolean {
                // Mirror Android SharedPreferences: update memory before disk persistence.
                apply()
                return !failCommit
            }

            override fun apply() {
                if (clearAll) map.clear()
                removals.forEach { map.remove(it) }
                map.putAll(pending)
            }
        }

        override fun registerOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) = Unit
        override fun unregisterOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener?) = Unit
    }

    @Test
    fun failedCommitRollsBackExistingAndNewlyAddedKeysInMemory() {
        val prefs = FailingCommitPreferences(
            mapOf(
                "overlay_bg_alpha" to 80,
                "translation_color" to 1,
            ),
        )
        prefs.failCommit = true

        assertThrows(IllegalStateException::class.java) {
            SettingsStore.writePreferenceWithRollback(prefs, listOf("overlay_bg_alpha")) {
                it.putInt("overlay_bg_alpha", 25)
            }
        }
        assertEquals(80, prefs.getInt("overlay_bg_alpha", -1))

        assertThrows(IllegalStateException::class.java) {
            SettingsStore.writePreferenceWithRollback(prefs, listOf("overlay_bg_color_hex")) {
                it.putString("overlay_bg_color_hex", "#1E293B")
            }
        }
        assertFalse(prefs.contains("overlay_bg_color_hex"))
        assertNull(prefs.getString("overlay_bg_color_hex", null))

        assertThrows(IllegalStateException::class.java) {
            SettingsStore.writePreferenceWithRollback(
                prefs,
                listOf("translation_color_hex", "translation_color"),
            ) {
                it.remove("translation_color_hex").putInt("translation_color", 3)
            }
        }
        assertEquals(1, prefs.getInt("translation_color", -1))
    }
}
