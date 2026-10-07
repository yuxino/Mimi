package app.yuxino.mimi.android

import java.util.Locale
import org.junit.Assert.*
import org.junit.Test

class LanguageDisplayTest {
    @Test fun expandedLanguagesHaveNamesInsteadOfAutomaticLabels() {
        assertEquals("French", explicitLanguageDisplayName("fr", Locale.ENGLISH))
        assertEquals("German", explicitLanguageDisplayName("de", Locale.ENGLISH))
        assertTrue(explicitLanguageDisplayName("zh_tw", Locale.ENGLISH).contains("Traditional"))
        assertEquals("Filipino", explicitLanguageDisplayName("tl", Locale.ENGLISH))
        assertEquals("Chinese and English (mixed)", explicitLanguageDisplayName("zh_en", Locale.ENGLISH))
        assertEquals("中文与英语（混合）", explicitLanguageDisplayName("zh_en", Locale.CHINESE))
        assertEquals("Shanghainese", explicitLanguageDisplayName("wuu", Locale.ENGLISH))
        assertEquals("zz", explicitLanguageDisplayName("zz", Locale.ENGLISH))
    }
}
