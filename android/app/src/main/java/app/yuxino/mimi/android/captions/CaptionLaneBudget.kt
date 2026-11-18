package app.yuxino.mimi.android.captions


/** Native visual-line allocation, using measured glyph heights rather than character counts. */
internal fun captionLaneBudget(height: Int, source: List<Int>, translation: List<Int>): Pair<Int, Int> {
    fun fit(heights: List<Int>, space: Int): Int = if (heights.isEmpty()) 0 else
        heights.indexOfLast { it <= space }.let { if (it < 0) 1 else it + 1 }
    if (source.isEmpty()) return 0 to fit(translation, height)
    if (translation.isEmpty()) return fit(source, height) to 0
    // Reserve two translated lines where one original line still fits. Equal
    // splitting can otherwise give the smaller reference font several lines
    // while leaving only a fragment of the actual reading target.
    val minimumTranslation = if (translation.size >= 2 && source[0] + translation[1] <= height) 2 else 1
    val originalSpace = height - translation[minimumTranslation - 1]
    var original = fit(source, minOf(height / 2, originalSpace))
    var translated = fit(translation, height - source[original - 1])
    original = fit(source, minOf(height - translation[translated - 1], originalSpace))
    translated = fit(translation, height - source[original - 1])
    return original to translated
}
