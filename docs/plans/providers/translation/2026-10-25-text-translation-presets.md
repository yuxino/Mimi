# Compatible text translation presets

The translation service picker keeps ChatMock last on desktop and Android.
Existing service marks and neutral menu styling remain; normalize the padded
DeepLX artwork and reduce the visual weight of the xAI favicon in shared pickers.

The OpenAI-compatible route offers 13 editable setup presets: OpenAI, DeepSeek,
Qwen, Gemini, Kimi, SiliconFlow, OpenRouter, Index-Translate, Tencent TokenHub,
Baidu Qianfan, Volcano Ark / Doubao, Ollama and LM Studio. Keep one protocol and
credential slot: presets are UI defaults, not new persisted provider identities.
Source labels, addresses and example models from `shared/provider-copy.json`;
the existing generator emits
desktop TypeScript and native Android Kotlin plus eight-language help.

Selecting a preset only changes the draft. Choosing a different destination preset clears the
typed key and disallows reusing the old saved key, including after switching back.
Equivalent destinations retain their key. Custom preserves the editable fields;
an explicit save and connection check remain separate user actions. Local HTTP
on Android still requires its existing explicit opt-in. Leave account-dependent
model IDs empty rather than guessing an available model or deployment. Existing
OpenAI/DeepSeek/Qwen examples remain; Index uses its official public model ID.

Text API presets use their own account, API product, region and key. In particular,
Tencent TokenHub, Baidu Qianfan and Volcano Ark do not reuse the credentials of
Mimi's existing integrated speech-translation services. The metadata catalog does
not change runtime request fields or establish real-provider acceptance.

Regression checks cover draft/save isolation, keyless local setup, equivalent
URLs, generated-output drift, native Spinner callback ownership and existing
keyboard navigation. Review menu geometry in all interface languages at minimum
and default widths in light/dark themes. Browser fixtures establish layout only;
native app/device and real provider checks remain separate evidence.

## Setup navigation

Each preset needs a clear next action beyond filling its address. The shared
catalog owns reviewed official key/guide URLs, cloud/local/keyless kind and a
localized short setup instruction. The existing generator emits the same
metadata for desktop and Android; keep account-dependent model IDs editable.

Put a compact Get API key action beside the key field and a setup guide beside
the model field on desktop. Native Android keeps its existing compact action and
help patterns. Cloud setup explains where to create the text-product key and
choose an enabled model. Local setup explains starting/loading the server;
Index explains its keyless public example. Keep extra detail in the existing
help controls and essential keyless/local choices readable in the form.

Selecting a service only changes a draft. Opening a guide requires an explicit
click, preserves edits and never saves, probes or reveals credentials. Desktop
IPC accepts a preset ID and a page enum, then resolves a fixed catalog URL in
native code; it never accepts an editable URL. Scope this command to Settings.
Android likewise opens generated public destinations, including when a pending
Spinner selection must be reconciled first. Report opening failures through
existing sanitized transient feedback.

Verify no navigation on selection, correct per-service destinations, local and
keyless actions, unavailable browser feedback, stale selection and draft/key
ownership. Review normal/narrow light/dark UI and retain real component
screenshots for the PR and chat, clearly labeling browser fixtures and native
acceptance limits. Preserve the currently running production application.
