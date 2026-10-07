# Service setup guide

[Back to README](../README.md) · [简体中文](provider-setup.zh-CN.md) · [Usage & FAQ](usage.md)

This guide explains where to enable each service, where to get its credentials, and what to enter in Mimi. Configure only the services you plan to use. For a first setup, you can start with Alibaba Cloud or Google Gemini, as recommended in the README.

Last checked: **2026-10-06**. The instructions primarily cover the current desktop app, with Android differences noted separately. Console menus, model access, trial allowances, and billing rules can change. Check the linked official documentation and your own account status.

## Find your service

| Service in Mimi | Setup / management | What you need |
| --- | --- | --- |
| [Alibaba Cloud](#alibaba-cloud) | [Model Studio API Keys](https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key) | A Model Studio API key for the Beijing region |
| [Volcano Engine](#volcano-engine) | [Doubao Speech API Key management](https://console.volcengine.com/speech/new/setting/apikeys?projectName=default) | An API key from the same project where Simultaneous Interpretation 2.0 is enabled |
| [Google Gemini](#google-gemini) | [Google AI Studio](https://aistudio.google.com/api-keys) | An API key with access to the Gemini Live translation model |
| [Tencent Cloud](#tencent-cloud) | [ASR console](https://console.cloud.tencent.com/asr) | AppID, SecretID, and SecretKey |
| [Baidu Translate](#baidu-translate) | [Machine Translation console](https://console.bce.baidu.com/ai-engine/machinetranslation/overview/index) | The application's AppID and API Key; enter the API Key in Mimi's AppKey field |
| [OpenAI Realtime](#openai-realtime) | [OpenAI API Keys](https://platform.openai.com/api-keys) | An API key for your OpenAI project |
| [Azure OpenAI](#azure-openai) | [Azure Portal](https://portal.azure.com/) / [Microsoft Foundry](https://ai.azure.com/) | Resource endpoint, translation deployment name, transcription deployment name, and API key |
| [xAI Grok](#xai-grok) | [xAI Console](https://console.x.ai/) | An xAI API key for the selected team |
| [Apple Speech](#apple-speech) | Choose and download a language inside Mimi | A supported Mac; no key is needed for recognition |
| [Custom Recognition (DashScope Compatible)](#custom-recognition) | Your DashScope-compatible service | Full WebSocket URL, recognition model, and API key |
| [Custom Recognition (OpenAI Realtime Compatible)](#custom-recognition) | Your Realtime-compatible service | Full WebSocket URL, recognition model, and API key |

Only setting up text translation? Go to [Apple Translation](#apple-translation), [DeepL](#deepl), [DeepLX](#deeplx), [ChatMock](#chatmock), [OpenAI-compatible API / Index-Translate](#openai-compatible), or [Original text only](#original-only). To check which models Mimi calls, see [Current models and request paths](#current-models).

<a id="current-models"></a>

## Current models and request paths

This table describes the desktop app's configured requests on the main branch as of the date above. Model IDs help you check access and understand usage. Built-in configurations select their models automatically; only Azure deployments and custom services require you to enter names. Providers can update models under the same ID, and an ID alone does not guarantee recognition quality or support for a particular language.

| Desktop service | Models or APIs used for recognition / original text and translation | How to select them in Mimi |
| --- | --- | --- |
| Alibaba Cloud | Recognition: `qwen-audio-3.0-asr-flash-streaming`; default text translation: `qwen-mt-lite` | Fixed built-in models; the text translation service can be changed independently |
| Google Gemini | `gemini-3.5-live-translate-preview`; original and translated text come from the same Live session, with no separate ASR model to enter | Fixed built-in model |
| Volcano Engine | Doubao Simultaneous Interpretation 2.0, resource ID `volc.service_type.10053`; `/api/v4/ast/v2/translate` | Built-in service endpoint; the request does not specify a public model ID |
| Tencent Cloud | ASR realtime speech translation; translation model: `hunyuan-translation-lite` | Fixed built-in configuration; the service provides recognition without a separate ASR model ID in the request |
| Baidu Translate | Realtime speech translation at `/ws/realtime_speech_trans` | Built-in service endpoint; the request does not specify a public model ID |
| OpenAI Realtime | Translation: `gpt-realtime-translate`; original transcription: `gpt-realtime-whisper` | Fixed built-in models using the dedicated Realtime translation endpoint |
| Azure OpenAI | Translation: your deployment of `gpt-realtime-translate`; original text: your compatible realtime transcription deployment | Enter both actual deployment names; Mimi does not choose the transcription model for you |
| xAI Grok | Voice: `grok-voice-latest`; original transcription: `grok-transcribe` | Fixed built-in models; turn-based translation |
| Apple Speech | macOS `SpeechTranscriber` with downloaded language packs; text translation is configured separately | Select a local language pack; there is no cloud model ID |
| Custom DashScope / OpenAI Realtime recognition | Recognition uses the model ID you enter; text translation is configured separately | Enter the endpoint, model, and recognition key yourself |

Alibaba Cloud on desktop currently uses `qwen-mt-lite` by default. It does not automatically switch to `qwen-mt-flash` or `qwen-mt-plus` for different recognition languages. The default recognition service receives a continuous audio stream over WebSocket, while remote independent text translation sends recognized text over HTTP. Apple Translation runs locally. Changing text translation does not change the recognition model.

Model and routing references: [Desktop service selection](../src-tauri/src/clients/translation_client.rs) · [Current translation modes](../src-tauri/src/core/configuration.rs) · [Provider protocols](../src-tauri/src/core/protocols/) · [Default Qwen-MT model](../src-tauri/src/core/protocols/qwen_mt.rs)

### Alibaba Cloud on Android

| Android mode | Current default models and requests |
| --- | --- |
| Built-in recognition + translation | Connects to `qwen3.5-livetranslate-flash-realtime` and selects `qwen3-asr-flash-realtime` within the same session for original transcription |
| Independent text translation / no translation | Uses `qwen3-asr-flash-realtime` for recognition only, then calls the selected text service if translation is needed |

This differs from the desktop Audio 3.0 + Qwen-MT setup. Some Android services also offer advanced endpoint / model overrides. Alibaba Cloud's independent text translation and no-translation modes still use `qwen3-asr-flash-realtime`, regardless of a saved model override. See [Android DashScopeEngine](../android/app/src/main/java/app/yuxino/mimi/android/provider/DashScopeEngine.kt) and [platform parity](development/platform-parity.md).

### Models used for independent text translation

| Text translation service | What Mimi selects |
| --- | --- |
| Alibaba Cloud (desktop default) | `qwen-mt-lite` |
| Apple Translation | macOS Translation framework and downloaded system resources; no cloud model ID |
| DeepL | Official text translation API; Mimi does not specify `model_type` or a model ID |
| DeepLX | The `/translate` service at the configured address; Mimi does not select its upstream model |
| ChatMock / OpenAI-compatible API | The model ID you enter; there is no shared built-in default model |
| Index-Translate example | `Index-Translate-35B-A3B`, configured through the OpenAI-compatible API option |
| Skip translation / no translation | No text translation model is called |

Request references: [DeepL](../src-tauri/src/core/protocols/deepl.rs) · [DeepLX](../src-tauri/src/core/protocols/deeplx.rs) · [OpenAI-compatible text translation](../src-tauri/src/core/protocols/openai_compatible.rs) · [Official Index example](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py)

## Complete the setup in Mimi

1. Stop subtitles, open **Settings → Speech & Translation**, and add a configuration for your service.
2. Get the credentials using the instructions below, enter each value in its corresponding field, and save. Do not combine credentials into JSON or paste an entire code example.
3. Select the recognition and translation languages, then run the connection check beside the configuration. When using independent text translation, check recognition and translation separately.
4. Play a short clip containing speech, then turn on **Live Subtitles** to confirm that text appears. A successful connection check confirms only that check; it does not verify every language or long-running sessions.

Cloud recognition receives audio from the selected source, and remote text translation receives the recognized text. Running two audio sources generates separate service usage for each. **Mimi being free does not mean the service APIs are free**: check service activation, available credits, and billing before starting. Connection checks may also generate a small amount of API usage. Do not expose real credentials in issues, logs, or screenshots.

<a id="alibaba-cloud"></a>

## Alibaba Cloud (Model Studio)

Links: [Beijing API Key management](https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key) · [Official activation guidance](https://help.aliyun.com/zh/model-studio/faq-about-alibaba-cloud-model-studio) · [Get an API key](https://help.aliyun.com/zh/model-studio/get-api-key)

1. Sign in to Alibaba Cloud and complete any real-name verification required by the console. In Model Studio, select **China North 2 (Beijing)** and follow the prompts to enable model services in that region.
2. Open API Key management in the Beijing region and create a key in the workspace you intend to use. You can also use an existing key.
3. If the workspace or key restricts model access, confirm that the models in the table below are authorized.
4. Add **Alibaba Cloud** in Mimi, enter the key under **Speech recognition → API Key**, and save. Default text translation uses the same Model Studio credentials.

| Mode | Current model access required |
| --- | --- |
| Desktop default recognition + translation | `qwen-audio-3.0-asr-flash-streaming` and `qwen-mt-lite` |
| Desktop with independent text translation or Skip translation | Recognition still uses `qwen-audio-3.0-asr-flash-streaming`; configure translation for the selected service |
| Android default recognition + translation | `qwen3.5-livetranslate-flash-realtime`, with `qwen3-asr-flash-realtime` transcribing the original in the same session |
| Android with independent text translation or no translation | `qwen3-asr-flash-realtime`; configure translation for the selected service |

Use a **Model Studio / DashScope API key**, not an Alibaba Cloud AccessKey ID / AccessKey Secret pair. The built-in desktop configuration uses `dashscope.aliyuncs.com` and has no region or endpoint field; keys from other regions cannot be used interchangeably. See the [official region guidance](https://help.aliyun.com/zh/model-studio/regions) and [platform parity](development/platform-parity.md) for the model differences between Mimi's two platforms.

Billing: the default desktop setup includes both speech recognition and text translation. Changing the text translation service still generates cloud recognition usage; whether it is charged depends on the recognition model's remaining trial allowance and billing status. Check [Model Studio pricing](https://help.aliyun.com/zh/model-studio/model-pricing) and the trial status of the models you use.

<a id="volcano-engine"></a>

## Volcano Engine (Doubao Speech)

Links: [New Doubao Speech console](https://console.volcengine.com/speech/new/overview?projectName=default) · [API Key management](https://console.volcengine.com/speech/new/setting/apikeys?projectName=default) · [Official new-console guide](https://docs.volcengine.com/docs/DoubaoVoice/QuickStartNewConsole?lang=zh)

1. Register or sign in to Volcano Engine and complete real-name verification.
2. Select a project in the **new Doubao Speech console**. The links above open `default`; switch projects first if you use another one.
3. In activation management (开通管理), confirm that **Simultaneous Interpretation 2.0** is enabled.
4. Create or copy a key in **API Key management for the same project**, then paste it into **Volcano Engine → API Key** in Mimi and save.

Mimi currently needs **one API key from the new Doubao Speech console** and uses the Simultaneous Interpretation 2.0 resource `volc.service_type.10053`. Do not enter a Volcano Ark model key or the old console's AppID / Access Token pair. Service activation, resource packages, and the key must belong to the corresponding project.

Official references: [API key usage](https://docs.volcengine.com/docs/DoubaoVoice/APIKeyUsage?lang=zh) · [Simultaneous Interpretation 2.0 integration](https://docs.volcengine.com/docs/DoubaoVoice/SimultaneousInterpretation20APIAccessDocumentation?lang=zh)

Billing: check the project's remaining trial allowance, resource packages, and postpaid settings in activation management. Usage may become postpaid when a trial runs out; successfully creating a key does not guarantee free usage. See the [billing overview](https://docs.volcengine.com/docs/DoubaoVoice/BillingOverview-15?lang=zh).

<a id="google-gemini"></a>

## Google Gemini

Links: [Google AI Studio API Keys](https://aistudio.google.com/api-keys) · [Official key guide](https://ai.google.dev/gemini-api/docs/api-key) · [Live translation documentation](https://ai.google.dev/gemini-api/docs/live-api/live-translate)

1. Sign in to Google AI Studio, accept the terms when prompted, and create or select an associated Google Cloud project.
2. Create a key for that project in API Keys and confirm that the project has access to the Live translation model.
3. Add **Google Gemini** in Mimi, enter the **API Key**, and save.

Mimi currently uses `gemini-3.5-live-translate-preview`. It requires a Gemini Developer API key; a normal Gemini website login or other Vertex AI credentials cannot be entered directly in this field. Preview model access, quotas, and network availability depend on the project you use.

Billing: check usage and the billing tier of the associated project in AI Studio, along with [Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing). Do not assume that another Gemini model's free allowance applies to Live translation.

<a id="tencent-cloud"></a>

## Tencent Cloud

Links: [ASR console](https://console.cloud.tencent.com/asr) · [Account Information / AppID](https://console.cloud.tencent.com/developer) · [CAM API Keys](https://console.cloud.tencent.com/cam/capi)

1. Sign in to Tencent Cloud, complete verification, and enable speech recognition by following the [official setup steps](https://cloud.tencent.com/document/product/1093/54362).
2. Confirm that the account has an available resource package for **large-model realtime speech translation** or has enabled postpaid billing for that service.
3. Find **AppID** on Account Information. Create a key on CAM API Keys and save **SecretID / SecretKey** when it is created. For an existing key, use the values you saved earlier; SecretKey cannot be retrieved again afterward. All three credentials must belong to the same account.
4. Enter the three values in their respective fields in Mimi's **Tencent Cloud** configuration and save.

Mimi uses the ASR [realtime speech translation WebSocket API](https://cloud.tencent.com/document/product/1093/127565) with `hunyuan-translation-lite` for translation. AppID is not UIN, and a single Hunyuan API key cannot replace SecretID / SecretKey. This setup does not require the separate Tencent Machine Translation (TMT) service or speech synthesis for subtitles.

Billing: as of the check date, [Tencent Cloud's billing documentation](https://cloud.tencent.com/document/product/1093/35686) states that **large-model realtime speech translation has no free allowance**, and postpaid billing is disabled by default. A free allowance for ordinary realtime ASR does not cover this service. Check the specific product so that your credentials have an active service to use.

<a id="baidu-translate"></a>

## Baidu Translate (Baidu AI Cloud)

Links: [Machine Translation console](https://console.bce.baidu.com/ai-engine/machinetranslation/overview/index) · [Official setup process](https://ai.baidu.com/ai-doc/MT/2l317egif) · [Realtime speech translation API](https://ai.baidu.com/ai-doc/MT/Sl9p2h5k9)

1. Sign in to Baidu AI Cloud, complete any verification required by the console, and create an application under Machine Translation or open an existing one.
2. Confirm that the application has **realtime speech translation** access and an available trial, resource package, or paid service.
3. Open the application details, copy **AppID** and **API Key** from the same application, enter them in Mimi as shown below, and save.

| Mimi field | Baidu application field |
| --- | --- |
| AppID | AppID |
| AppKey | **API Key** |

Mimi's `AppKey` corresponds to `app_key` in the realtime API, which the official documentation defines as the application's **API Key**. Do not enter a Secret Key, OAuth Access Token, or a key for a different translation product. You do not need to request an OAuth token yourself.

Billing: see [realtime speech translation pricing](https://ai.baidu.com/ai-doc/MT/Tl9pjqsym). Activation and allowances for text translation or ordinary speech recognition do not replace realtime speech translation access.

<a id="openai-realtime"></a>

## OpenAI Realtime

Links: [OpenAI API Keys](https://platform.openai.com/api-keys) · [Realtime translation guide](https://developers.openai.com/api/docs/guides/realtime-translation)

1. Sign in to the OpenAI developer platform and select the project you want to use.
2. Set up API billing for the project and confirm access to Realtime translation and the required transcription model.
3. Create a project key in API Keys, paste it into **OpenAI Realtime → API Key** in Mimi, and save.

Mimi currently calls `gpt-realtime-translate` and uses `gpt-realtime-whisper` for the original text. Enter an OpenAI API key, not a ChatGPT password or login token. **ChatGPT subscriptions and API billing are separate**; see the [official explanation](https://help.openai.com/en/articles/9039756-managing-billing-for-chatgpt-and-the-api-platform).

Billing: check [API pricing](https://developers.openai.com/api/docs/pricing) and project usage. Creating a key alone does not mean model access or API credits are ready.

<a id="azure-openai"></a>

## Azure OpenAI

Links: [Azure Portal](https://portal.azure.com/) · [Microsoft Foundry](https://ai.azure.com/) · [Official Realtime WebSocket guide](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/realtime-audio-websockets#translate-audio-in-real-time)

1. Create or select an **Azure OpenAI resource** in an active Azure subscription and a region that supports the required models.
2. Deploy `gpt-realtime-translate` in that resource. Following the [official realtime transcription guidance](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/gpt-realtime-whisper), also prepare a compatible deployment for original transcription, such as `gpt-realtime-whisper`. Confirm model availability, region, and access permissions. If the required model is unavailable, do not substitute an ordinary chat deployment.
3. Record **both actual deployment names**, then copy the resource endpoint and key from its **Keys and Endpoint** page.
4. Fill in Mimi's **Azure OpenAI** configuration using the table below and save.

| Mimi field | What to enter |
| --- | --- |
| Azure Resource Endpoint | The resource root URL, such as `https://YOUR-RESOURCE.openai.azure.com` |
| Translation Deployment | The actual deployment name for `gpt-realtime-translate` in this resource |
| Transcription Deployment | The actual deployment name for a compatible realtime transcription model in the same resource |
| API Key | The key for this Azure OpenAI resource |

**A deployment name can differ from the model name.** Copy the name from the deployment list. Enter only the resource root URL; do not append `/openai/v1` or an API path. Mimi currently accepts resource domains ending in `*.openai.azure.com`, `*.openai.azure.cn`, or `*.openai.azure.us`. An accepted domain does not mean the corresponding cloud or region offers the required models.

This service connects to **Azure OpenAI**, not an Azure Speech resource. A normal OpenAI API key, Speech key, or Azure subscription ID cannot replace the resource key here. Usage is billed to the Azure subscription; see [Azure OpenAI pricing](https://azure.microsoft.com/en-us/pricing/details/cognitive-services/openai-service/).

<a id="xai-grok"></a>

## xAI Grok

Links: [xAI Console](https://console.x.ai/) · [Official quickstart](https://docs.x.ai/developers/quickstart) · [Voice API](https://docs.x.ai/developers/model-capabilities/audio/speech-to-speech)

1. Sign in to xAI Console and select or create a team.
2. Check the team's API usage, billing settings, and Voice model access, then create an API key.
3. Add **xAI Grok** in Mimi, enter the **API Key**, and save.

Mimi currently uses `grok-voice-latest` with `grok-transcribe` for original text, providing turn-based speech translation. Enter only the team's xAI API key here. Check allowances and charges in the console and [xAI API pricing](https://docs.x.ai/developers/pricing); a consumer subscription alone does not establish API availability.

<a id="apple-speech"></a>

## Apple Speech (local recognition on Mac)

Apple Speech is available only on a Mac with **Apple silicon, macOS 26 or later, and an available system recognition engine**. Recognition requires no cloud service account or API key.

1. Stop subtitles and add an **Apple Speech** configuration.
2. Choose a language under **Recognition Language**. If it is missing, choose **Download and use** beside its status. Apple provides the packs, so this step may need an internet connection; Mimi selects the language when the download finishes.
3. For an already downloaded language, choose **Set recognition language**, then start subtitles. You do not need to find a language-pack setting in macOS System Settings. Download or save failures keep their error and retry action beside the selected language.
4. Select **No translation (original only)** if you only need the original text. To get translated text, configure one of the text translation services below separately.

Automatic language detection is not supported. The settings list shows this Mac’s supported languages and download status; a language can be applied only when its resources are ready and the translation configuration supports it. Tray and floating controls use that saved source and offer only ready, compatible choices. Apple Speech does not provide text translation itself; it can use the separate Apple Translation service. Using a remote translator still sends text to that service. See [platform parity](development/platform-parity.md) for platform requirements and [language setup](speech-language-setup.md#english) for language parameters.

<a id="custom-recognition"></a>

## Custom speech recognition

These two options are for users who already have a compatible speech service. They do not share a single service activation website.

1. Based on your service's official protocol, add **Custom Recognition (DashScope Compatible)** for the DashScope protocol or **Custom Recognition (OpenAI Realtime Compatible)**.
2. Obtain the **full WebSocket URL, deployed recognition model ID, and API key** from the service provider. All three fields are required in the current Mimi app; the optional-key rule for text translation does not apply here.
3. Use `wss://` for remote addresses; `ws://` is allowed only for loopback addresses on the same device. Do not include a username, password, query parameters, or fragment in the URL. Mimi adds the connection parameters required for OpenAI transcription.
4. Enable **Skip translation** first, check recognition, and test a short speech sample. Once recognition works, configure independent text translation.

A normal webpage URL, HTTP file-transcription endpoint, or Chat Completions endpoint cannot serve as a realtime speech recognition endpoint. See [speech service and language setup](speech-language-setup.md#english) for examples, protocol requirements, and the meaning of **Service default** for languages. Activation, deployment fees, and local running costs depend on the service you use.

<a id="text-translation"></a>

## Independent text translation

On desktop, **Alibaba Cloud, Apple Speech, and both custom recognition options** support independent text translation. Other built-in speech services use their own translation pipelines. Android currently offers independent text translation only within Alibaba Cloud configurations.

Complete the **Speech recognition** section first, then choose a service under **Text translation**. When you select DeepL, DeepLX, ChatMock, or an OpenAI-compatible API, Mimi does not use the recognition key in place of translation credentials. Default Alibaba Cloud translation is covered in the [Alibaba Cloud section](#alibaba-cloud).

<a id="apple-translation"></a>

### Apple Translation

No service account, API key or text proxy is needed. This service requires a supported Apple silicon Mac running macOS 26 or later; the system supplies the actual supported languages.

1. Select **Apple Translation** under **Text translation** and save the configuration.
2. Choose an explicit recognition language and translation target. If the pair is not ready, click **Download or enable languages**, confirm in Apple’s interface, and wait for completion.
3. Once Mimi shows **Ready**, run the text translation check or start subtitles. Leaving Apple’s interface before downloading keeps the pair unready and allows retry.

Translation resources are separate from Apple Speech recognition resources; existing resources are reused. Selecting the service, checking connections and starting subtitles never initiate hidden downloads. Text translation stays on the Mac; cloud recognition still sends audio to the selected recognition service. Apple Translation is not available on Android.

<a id="deepl"></a>

### DeepL

Links: [DeepL API plans](https://www.deepl.com/en/products/api) · [Account API Keys & Limits](https://app.deepl.com/your-account/keys) · [Official key guide](https://support.deepl.com/hc/en-us/articles/360020695820-API-key-for-DeepL-API)

1. Sign up for a **DeepL API** plan that suits your needs, then create or copy a key from API Keys & Limits in your account.
2. Select **DeepL** under **Text translation** in Mimi, enter the full **DeepL API key**, and save.
3. Run the text translation connection check. Mimi automatically selects the Free endpoint for keys ending in `:fx` and the Pro endpoint for other keys. No service address or model name is needed here.

If you cannot find a key, first confirm that you have an API plan. A subscription to the translation website or app is not an API plan. See [DeepL authentication](https://developers.deepl.com/docs/getting-started/auth) for endpoint rules, and check allowances and fees in your account.

<a id="deeplx"></a>

### DeepLX

Links: [Upstream DLX project (formerly DeepLX)](https://github.com/OwO-Network/DLX) · [Deployment guide](https://deeplx.owo.network/install/) · [API format](https://deeplx.owo.network/endpoints/free.html)

1. Deploy the service by following the upstream instructions, or obtain a service address and access token from your chosen provider.
2. Select **DeepLX** under **Text translation** and enter the service address. If the instance requires a Bearer token, enter it in **Access token (optional)**; otherwise leave it blank.
3. Save and check translation. Mimi appends `/translate` automatically and also accepts addresses that already end in `/translate`.

Local example: `http://127.0.0.1:1188/translate`. Use HTTPS for remote or LAN servers. Enter the token in its dedicated field, not in URL query parameters. DeepLX is an independent third-party service, not the official DeepL API; do not use an official DeepL API key as an instance token. Self-hosting does not mean the translation model runs locally. Costs and availability depend on the deployment and its upstream service.

<a id="chatmock"></a>

### ChatMock

Link: [ChatMock upstream project and installation instructions](https://github.com/RayBytes/ChatMock)

1. Install ChatMock by following its upstream instructions, **sign in to your account within ChatMock**, and start the service.
2. Select **ChatMock** under **Text translation** in Mimi. The default address for a service on the same computer is `http://127.0.0.1:8000/v1`.
3. Enter an actual model ID returned by the service's `/v1/models` endpoint. Leave Mimi's API key field blank if the service does not require Bearer authentication.
4. Save and check translation. For ChatMock on another computer or server, enter its HTTPS address.

Keep account sign-in inside ChatMock. **Do not enter a ChatGPT password or login token in Mimi.** A local address only means the interface is local; the models are still called online through the signed-in account. Account benefits, limits, and service availability depend on that account's actual status. The service must return final text; compatibility options include `--reasoning-compat legacy --reasoning-summary none`.

<a id="openai-compatible"></a>

### OpenAI-compatible API and Index-Translate

Enable a text model with your chosen provider and obtain its **service address, model ID, and an API key if required**. Select **OpenAI-compatible API** under **Text translation** in Mimi, enter each value, and save.

The endpoint must support non-streaming Chat Completions and return `choices[0].message.content`. The service address can be an API base path such as `/v1` or the full `/chat/completions` path. Mimi appends only `/chat/completions`; **it does not add `/v1` automatically**. For example, `https://example.com/v1` produces requests to `https://example.com/v1/chat/completions`. Use HTTPS for remote services; HTTP is allowed for loopback addresses. Addresses cannot contain a username, password, query parameters, or fragment.

As of the check date, Bilibili's [official Index-Translate example](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py) provides this free public text translation endpoint without authentication:

| Mimi field | What to enter |
| --- | --- |
| Service address | `https://index-translate.bilibili.com/v1` |
| Model name | `Index-Translate-35B-A3B` |
| API key (optional) | Leave blank |

If a key was previously saved for this same address, choose **Remove translation key** and save. Simply leaving the input blank may retain the old key. Continued availability of the free endpoint depends on the upstream service. Index-Translate translates text only; cloud speech recognition may still incur charges.

<a id="original-only"></a>

### Original text only

In configurations that support independent text translation, enable **Skip translation** to stop sending text translation requests. Apple Speech and custom recognition also offer **No translation (original only)**. On Android, select **No translation (original only)** in Alibaba Cloud's text translation options.

**Subtitle display → Original only** changes only what is displayed; it does not disable translation requests. To reduce translation calls, use Skip translation or No translation as described above. Cloud recognition usage continues to accrue.

## Android setup notes

Android's eight built-in cloud speech services use the corresponding credentials described above. Android has no Apple Speech or custom recognition configuration options. See the [Alibaba Cloud table](#alibaba-cloud) for its model differences. Use other advanced endpoint or model overrides only when the service documentation explicitly confirms compatibility.

On a phone, `127.0.0.1` refers to the phone itself. To connect to ChatMock or DeepLX on a computer, normally use that computer's HTTPS service address. Local HTTP on Android requires explicit opt-in. For emulator and USB development connections, see the [Android guide](../android/README.md#independent-text-translation).

## Still not working?

| Problem | Check first |
| --- | --- |
| Authentication failure / 401 / 403 | Check that the key belongs to the correct product and is active, and that the project, account, region, and model permissions match |
| A key is saved, but the service reports that it is inactive or has insufficient quota | Confirm that you enabled the specific product described above; check trial expiry, resource-package eligibility, and postpaid activation |
| Model / deployment not found | Check model access for the account and region. Azure requires the actual deployment name; custom services require the actual model ID |
| Recognition produces original text but no translation | Check text translation separately. Confirm that Skip translation is off, No translation is not selected, and the target language is correct |
| Configuration saved, but no subtitles appear | Run the connection check, then play speech and start subtitles. Saving a configuration does not start audio capture |
| One particular language fails | Check the specific model and API's support in [speech and language setup](speech-language-setup.md#english). An available UI option does not mean your account has access |

If the problem continues, report it in [Issues](https://github.com/yuxino/mimi/issues) with your Mimi version, operating system, service name, language pair, and sanitized error details. For audio permissions and installation problems, see the [usage guide](usage.md).
