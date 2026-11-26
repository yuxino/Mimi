package app.yuxino.mimi.android.captions

/** Pure RGB/HEX color helpers shared by Android settings, picker dialog, and overlay rendering. */
internal object CaptionColorMath {
    const val DEFAULT_BG_COLOR: Int = 0xFF000000.toInt()
    val BG_COLOR_PRESETS: List<Int> = listOf(
        0xFF000000.toInt(),
        0xFF1C1C1E.toInt(),
        0xFF1E293B.toInt(),
        0xFF26211D.toInt(),
        0xFFFFFFFF.toInt(),
    )

    /** Accepts `#RRGGBB` or `RRGGBB` (case-insensitive) and returns an opaque ARGB int. */
    fun parseHexColor(raw: String?): Int? {
        val trimmed = raw?.trim().orEmpty()
        if (trimmed.isEmpty()) return null
        val hex = if (trimmed.startsWith("#")) trimmed.substring(1) else trimmed
        if (hex.length != 6) return null
        var rgb = 0
        for (ch in hex) {
            val digit = when (ch) {
                in '0'..'9' -> ch - '0'
                in 'a'..'f' -> ch - 'a' + 10
                in 'A'..'F' -> ch - 'A' + 10
                else -> return null
            }
            rgb = (rgb shl 4) or digit
        }
        return (0xFF shl 24) or rgb
    }

    /** Formats the RGB channels as uppercase `#RRGGBB`. */
    fun formatHexColor(argb: Int): String =
        String.format("#%06X", argb and 0xFFFFFF)

    /** Matches desktop threshold: light, sufficiently opaque surfaces switch chrome to dark ink. */
    fun isLightSurface(argb: Int, alphaPercent: Int): Boolean {
        if (alphaPercent.coerceIn(0, 100) < 35) return false
        val red = (argb ushr 16) and 0xFF
        val green = (argb ushr 8) and 0xFF
        val blue = argb and 0xFF
        return (0.2126 * red + 0.7152 * green + 0.0722 * blue) / 255.0 > 0.45
    }

    fun red(argb: Int): Int = (argb ushr 16) and 0xFF
    fun green(argb: Int): Int = (argb ushr 8) and 0xFF
    fun blue(argb: Int): Int = argb and 0xFF
}
