package app.yuxino.mimi.android

import androidx.lifecycle.ViewModel
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider

/** Editor RAM only. Never put these raw drafts in a Bundle or saved-state handle. */
internal class ServiceEditorDraftViewModel : ViewModel() {
    private data class Identity(val profileId: String?, val provider: ServiceProvider)
    private var identity: Identity? = null
    private var draft: ServiceEditorDraft? = null

    fun providerFor(profileId: String?): ServiceProvider? = identity?.takeIf { it.profileId == profileId }?.provider

    fun restore(profileId: String?, provider: ServiceProvider): ServiceEditorDraft? {
        val requested = Identity(profileId, provider)
        if (identity != requested) { clearDraft(); identity = requested }
        return draft
    }

    fun remember(profileId: String?, provider: ServiceProvider, next: ServiceEditorDraft) {
        restore(profileId, provider)
        draft = next
    }

    fun clearDraft() { draft = null; identity = null }
    override fun onCleared() { clearDraft() }
}

// Plain objects deliberately have no generated secret-containing toString and
// hold no Activity, View, saved credential configuration or network request.
internal class ServiceEditorDraft(
    val name: String,
    val speechValues: Map<String, String>,
    val workspace: String?,
    val alibabaMode: Int?,
    val translation: TextTranslationEditorDraft?,
)

internal class TextTranslationEditorDraft(
    val selected: TextTranslationProvider,
    val qwenMtModel: String,
    val routes: Map<TextTranslationProvider, TextTranslationRouteDraft>,
)

internal class TextTranslationRouteDraft(
    val endpoint: String,
    val model: String,
    val key: String,
    val localHttp: Boolean,
    val forgetKey: Boolean,
)
