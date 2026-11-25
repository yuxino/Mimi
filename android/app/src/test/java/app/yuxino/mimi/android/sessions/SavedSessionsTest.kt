package app.yuxino.mimi.android.sessions

import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.io.IOException

class SavedSessionsTest {
    @Test fun paginationUsesSharedNewestFirstOneBasedIndices() {
        fun page(total: Int, number: Int): SavedSessionPage {
            val indices = (total downTo 1).drop(number * 30).take(30)
            return SavedSessionPage(total, number, indices.map { SavedSessionCaption("system", "", "", 0, it) })
        }
        assertFalse(page(0, 0).hasNext)
        assertFalse(page(1, 0).hasNext)
        assertFalse(page(30, 0).hasNext)
        assertTrue(page(31, 0).hasNext)
        assertFalse(page(31, 1).hasNext)
        assertTrue(page(60, 0).hasNext)
        assertFalse(page(60, 1).hasNext)
    }
    @Test fun formatterFailurePreservesExistingDestination() {
        val destination = File.createTempFile("mimi-existing-document", ".txt").apply { writeText("existing synthetic document") }
        try {
            assertThrows(IOException::class.java) {
                exportSavedSession({ throw IOException("synthetic") }, { destination.writeText("must not write") })
            }
            assertEquals("existing synthetic document", destination.readText())
        } finally { destination.delete() }
    }
    @Test fun failedCopyRemovesOnlyTemporaryAndKeepsSelectedDestination() {
        val temporary = File.createTempFile("mimi-export-test", ".txt")
        val destination = File.createTempFile("mimi-existing-document", ".txt").apply { writeText("existing synthetic document") }
        try {
            assertThrows(IOException::class.java) {
                exportSavedSession({ temporary }, {
                    destination.writeText("partial synthetic export")
                    throw IOException("synthetic")
                })
            }
            assertFalse(temporary.exists())
            assertEquals("partial synthetic export", destination.readText())
        } finally { temporary.delete(); destination.delete() }
    }
    @Test fun successfulCopyRemovesOnlyTemporary() {
        val temporary = File.createTempFile("mimi-export-test", ".txt").apply { writeText("synthetic fixture") }
        var copied = false
        exportSavedSession({ temporary }, { copied = it.readText() == "synthetic fixture" })
        assertTrue(copied); assertFalse(temporary.exists())
    }
    @Test fun batchSelectionTogglesSelectsAllAndPrunesMissingIds() {
        val available = setOf("a", "b", "c")
        val inactive = SavedSessionSelection().toggle("a", available)
        assertFalse(inactive.active)
        assertTrue(inactive.ids.isEmpty())

        var selection = SavedSessionSelection(active = true)
        selection = selection.toggle("a", available)
        assertEquals(setOf("a"), selection.ids)
        selection = selection.toggle("missing", available)
        assertEquals(setOf("a"), selection.ids)
        selection = selection.toggleAll(available)
        assertEquals(available, selection.ids)
        selection = selection.toggleAll(available)
        assertTrue(selection.ids.isEmpty())
        selection = selection.toggle("a", available).toggle("b", available)
        selection = selection.prune(setOf("b", "c"))
        assertTrue(selection.active)
        assertEquals(setOf("b"), selection.ids)
        val emptied = selection.prune(emptySet())
        assertFalse(emptied.active)
        assertTrue(emptied.ids.isEmpty())
    }
}
