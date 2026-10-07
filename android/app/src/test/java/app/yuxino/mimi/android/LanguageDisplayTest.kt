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
        assertEquals(R.string.lang_chinese_english, languageNameResource("zh_en"))
        assertEquals(R.string.lang_shanghainese, languageNameResource("wuu"))
        assertEquals("zz", explicitLanguageDisplayName("zz", Locale.ENGLISH))
    }
}
