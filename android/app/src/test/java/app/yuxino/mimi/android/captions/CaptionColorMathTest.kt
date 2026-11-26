package app.yuxino.mimi.android.captions

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CaptionColorMathTest {
    @Test
    fun parsesAndFormatsHexWithOrWithoutHash() {
        assertEquals(0xFF1E293B.toInt(), CaptionColorMath.parseHexColor("#1e293b"))
        assertEquals(0xFF1E293B.toInt(), CaptionColorMath.parseHexColor(" 1e293b "))
        assertEquals(0xFF000000.toInt(), CaptionColorMath.parseHexColor("#000000"))
        assertEquals(0xFFFFFFFF.toInt(), CaptionColorMath.parseHexColor("ffffff"))
        assertEquals("#1E293B", CaptionColorMath.formatHexColor(0xFF1E293B.toInt()))
        assertEquals("#000000", CaptionColorMath.formatHexColor(0xFF000000.toInt()))
    }

    @Test
    fun rejectsPartialOrInvalidHex() {
        for (invalid in listOf(null, "", "#", "#fff", "12345", "#12345678", "#GG0011", "black")) {
            assertNull(CaptionColorMath.parseHexColor(invalid))
        }
    }

    @Test
    fun detectsLightOpaqueBackgroundSurfaces() {
        assertFalse(CaptionColorMath.isLightSurface(0xFF000000.toInt(), 80))
        assertFalse(CaptionColorMath.isLightSurface(0xFF1E293B.toInt(), 90))
        assertTrue(CaptionColorMath.isLightSurface(0xFFFFFFFF.toInt(), 80))
        assertFalse(CaptionColorMath.isLightSurface(0xFFFFFFFF.toInt(), 25))
    }
}
