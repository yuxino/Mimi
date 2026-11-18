package app.yuxino.mimi.android.services.setup

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.provider.TextTranslationProvider

internal enum class TextTranslationSetupPage { API_KEY, GUIDE }

/** Only catalog-owned public URLs may leave the editor; editable addresses are never browser targets. */
internal fun textTranslationSetupUrl(presetId: String, page: TextTranslationSetupPage): String? {
    val preset = TextTranslationPresets.entries.firstOrNull { it.id == presetId } ?: return null
    return when (page) {
        TextTranslationSetupPage.API_KEY -> preset.apiKeyUrl.takeIf { preset.setupKind == "cloud" && it.isNotBlank() }
        TextTranslationSetupPage.GUIDE -> preset.guideUrl.takeIf(String::isNotBlank)
    }
}

internal fun openTextTranslationSetup(activity: AppCompatActivity, presetId: String, page: TextTranslationSetupPage) {
    val url = textTranslationSetupUrl(presetId, page) ?: return
    openSetupUrl(activity, url)
}

internal fun serviceSetupUrl(serviceId: String, page: TextTranslationSetupPage): String? {
    val setup = ServiceSetups.entries.firstOrNull { it.id == serviceId } ?: return null
    return when (page) {
        TextTranslationSetupPage.API_KEY -> setup.apiKeyUrl
        TextTranslationSetupPage.GUIDE -> setup.guideUrl
    }.takeIf(String::isNotBlank)
}

internal fun textTranslationServiceSetupId(provider: TextTranslationProvider): String? = when (provider) {
    TextTranslationProvider.XAI -> "xAIRealtime"
    TextTranslationProvider.DEEPL -> "deepL"
    else -> null
}

internal fun openServiceSetup(activity: AppCompatActivity, serviceId: String, page: TextTranslationSetupPage) {
    val url = serviceSetupUrl(serviceId, page) ?: return
    openSetupUrl(activity, url)
}

private fun openSetupUrl(activity: AppCompatActivity, url: String) {
    try { activity.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
    catch (_: ActivityNotFoundException) { Toast.makeText(activity, R.string.guide_no_browser, Toast.LENGTH_SHORT).show() }
    catch (_: SecurityException) { Toast.makeText(activity, R.string.translation_setup_open_failed, Toast.LENGTH_SHORT).show() }
}
