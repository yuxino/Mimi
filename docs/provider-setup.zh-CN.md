# 服务开通指南

[返回 README](readme/zh-CN.md) · [English](provider-setup.md) · [使用与常见问题](usage.zh-CN.md)

这份指南说明各项服务去哪里开通、凭证从哪里拿，以及在 Mimi 里填写什么。只需配置你要使用的服务，不必全部开通。首次使用可以从 README 推荐的阿里云或 Google Gemini 开始。

核对日期：**2026-10-06**。下文以当前桌面版为主，Android 差异单独注明。控制台菜单、模型开放范围、试用额度和计费规则可能变化，请以对应官方文档和自己账号的状态为准。

## 先找到你的服务

| Mimi 中的服务 | 开通 / 管理入口 | 需要准备 |
| --- | --- | --- |
| [Alibaba Cloud / 阿里云](#alibaba-cloud) | [百炼 API Key](https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key) | 北京地域的百炼 API Key |
| [Volcano Engine / 火山引擎](#volcano-engine) | [豆包语音 API Key 管理](https://console.volcengine.com/speech/new/setting/apikeys?projectName=default) | 已开通同声传译 2.0 的同一项目内的 API Key |
| [Google Gemini](#google-gemini) | [Google AI Studio](https://aistudio.google.com/api-keys) | 可调用 Gemini Live 翻译模型的 API Key |
| [Tencent Cloud / 腾讯云](#tencent-cloud) | [ASR 控制台](https://console.cloud.tencent.com/asr) | AppID、SecretID、SecretKey |
| [Baidu Translate / 百度](#baidu-translate) | [机器翻译控制台](https://console.bce.baidu.com/ai-engine/machinetranslation/overview/index) | 应用的 AppID、API Key（填入 Mimi 的 AppKey） |
| [OpenAI Realtime](#openai-realtime) | [OpenAI API Keys](https://platform.openai.com/api-keys) | OpenAI 项目的 API Key |
| [Azure OpenAI](#azure-openai) | [Azure Portal](https://portal.azure.com/) / [Microsoft Foundry](https://ai.azure.com/) | 资源端点、翻译部署名、转写部署名、API Key |
| [xAI Grok](#xai-grok) | [xAI Console](https://console.x.ai/) | 所选团队的 xAI API Key |
| [Apple Speech](#apple-speech) | 在 Mimi 内选择语言并下载 | 符合条件的 Mac；识别无需 Key |
| [自定义识别（阿里云兼容）](#custom-recognition) | 你使用的 DashScope 兼容服务 | 完整 WebSocket URL、识别模型、API Key |
| [自定义识别（OpenAI 实时兼容）](#custom-recognition) | 你使用的 Realtime 兼容服务 | 完整 WebSocket URL、识别模型、API Key |

只想配置文字翻译？跳到 [Apple 翻译](#apple-translation)、[DeepL](#deepl)、[DeepLX](#deeplx)、[ChatMock](#chatmock)、[OpenAI 兼容接口 / Index-Translate](#openai-compatible)，或[仅显示原文](#original-only)。想确认具体调用哪个模型，见[当前模型与调用方式](#current-models)。

<a id="current-models"></a>

## 当前模型与调用方式

下表按核对日的主分支整理，列出桌面端当前配置的实际调用方式。模型 ID 用于确认权限和理解用量；内置配置会自动选用，只有 Azure 部署和自定义服务需要自己填写名称。服务商可更新同名模型，模型 ID 本身不能保证识别质量或某个语言的可用性。

| 桌面服务 | 识别 / 原文与翻译所用模型或接口 | 在 Mimi 中如何选择 |
| --- | --- | --- |
| Alibaba Cloud | 识别：`qwen-audio-3.0-asr-flash-streaming`；默认文字翻译：`qwen-mt-lite` | 内置固定；可单独更换文字翻译服务 |
| Google Gemini | `gemini-3.5-live-translate-preview`；原文和译文来自同一 Live 会话，没有另填 ASR 模型 | 内置固定 |
| Volcano Engine | 豆包同声传译 2.0，资源 ID `volc.service_type.10053`；`/api/v4/ast/v2/translate` | 内置服务接口；请求未指定公开模型 ID |
| Tencent Cloud | ASR 实时语音翻译；翻译模型 `hunyuan-translation-lite` | 内置固定；识别由该服务提供，未单独指定 ASR 模型 ID |
| Baidu Translate | 实时语音翻译 `/ws/realtime_speech_trans` | 内置服务接口；请求未指定公开模型 ID |
| OpenAI Realtime | 翻译：`gpt-realtime-translate`；原文转写：`gpt-realtime-whisper` | 内置固定，使用专用 Realtime 翻译接口 |
| Azure OpenAI | 翻译：用户部署的 `gpt-realtime-translate`；原文：用户指定的兼容实时转写部署 | 填写两个实际部署名；Mimi 不替用户选择转写模型 |
| xAI Grok | Voice：`grok-voice-latest`；原文转写：`grok-transcribe` | 内置固定；回合式翻译 |
| Apple Speech | macOS `SpeechTranscriber` 与已下载语言包；文字翻译单独配置 | 选择本机语言包；无云端模型 ID |
| 自定义 DashScope / OpenAI Realtime 识别 | 识别使用用户填入的模型 ID；文字翻译单独配置 | 端点、模型与识别 Key 均由用户填写 |

桌面阿里当前默认使用 `qwen-mt-lite`，不会因识别语言不同自动切换到 `qwen-mt-flash` 或 `qwen-mt-plus`。默认识别经 WebSocket 持续送入音频，远程独立文字翻译通过 HTTP 发送识别文字，Apple 翻译则在本地执行；更换文字翻译不会更换识别模型。

模型与路由依据：[共享服务选择](../shared/mimi-runtime/src/clients/translation_client.rs) · [当前翻译模式](../shared/mimi-runtime/src/core/configuration.rs) · [各家协议实现](../shared/mimi-runtime/src/core/protocols/) · [Qwen-MT 默认模型](../shared/mimi-runtime/src/core/protocols/qwen_mt.rs)

### Android 的阿里云链路

| Android 使用方式 | 当前默认模型与调用方式 |
| --- | --- |
| 内置识别 + 翻译 | 共享 Audio3 识别（`qwen-audio-3.0-asr-flash-streaming`）＋Qwen-MT，默认 Lite |
| 独立文字翻译 / 不翻译 | 使用同一 Audio3 识别；启用翻译时调用所选文字服务 |

Android 正式会话与桌面使用同一个共享 Rust 服务工厂。旧 Kotlin 传输类保留用于离线回归，其中的模型不是当前正式链路的默认模型。保存的高级设置不会覆盖内置链路不支持的端点或模型。实现见[共享服务选择](../shared/mimi-runtime/src/clients/translation_client.rs)和[平台差异](development/platform-parity.md)。

### 独立文字翻译用什么模型

| 文字翻译服务 | Mimi 实际选择 |
| --- | --- |
| Alibaba Cloud（桌面与 Android 默认） | `qwen-mt-lite` |
| Apple 翻译 | macOS Translation 框架与已下载的系统资源；无云端模型 ID |
| DeepL | 官方文本翻译 API；Mimi 不指定 `model_type` 或模型 ID |
| DeepLX | 配置地址提供的 `/translate` 服务；Mimi 不选择其上游模型 |
| ChatMock / OpenAI 兼容接口 | 使用用户填写的模型 ID，没有统一内置默认模型 |
| Index-Translate 示例 | `Index-Translate-35B-A3B`，通过 OpenAI 兼容接口配置 |
| 跳过翻译 / 不翻译 | 不调用文字翻译模型 |

请求依据：[DeepL](../src-tauri/src/core/protocols/deepl.rs) · [DeepLX](../src-tauri/src/core/protocols/deeplx.rs) · [OpenAI 兼容文字翻译](../src-tauri/src/core/protocols/openai_compatible.rs) · [Index 官方示例](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py)

## 在 Mimi 中完成配置

1. 停止字幕，打开「**设置 → 语音与翻译**」，添加对应服务的配置。
2. 按下文获取凭证，在对应字段分别填写并保存。凭证不需要拼成 JSON，也不要把整段示例代码粘进去。
3. 选择识别语言和翻译语言，运行配置旁的连接检查。使用独立文字翻译时，分别检查识别与翻译。
4. 播放一小段包含人声的内容，再开启「实时字幕」确认实际出字。连接检查通过只说明该次检查成功，不代表所有语言或长时间使用都已验证。

云端识别会接收所选音源的音频，远程文字翻译会接收识别文字。两路音源同时运行会分别产生服务用量。**Mimi 免费不等于服务 API 免费**：开始前查看服务是否开通、可用额度和付费方式；连接检查也可能产生少量 API 用量。不要在 Issue、日志或截图中公开真实凭证。

<a id="alibaba-cloud"></a>

## Alibaba Cloud（阿里云百炼）

入口：[北京地域 API Key 管理](https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key) · [官方开通说明](https://help.aliyun.com/zh/model-studio/faq-about-alibaba-cloud-model-studio) · [获取 API Key](https://help.aliyun.com/zh/model-studio/get-api-key)

1. 登录阿里云，按控制台要求完成实名认证，在百炼中选择**华北 2（北京）**，按提示开通该地域的模型服务。
2. 在北京地域进入 API Key 管理，在要使用的业务空间下创建 Key；已有 Key 也可以使用。
3. 如果业务空间或 Key 限制了可调用模型，确认下表中的实际模型已获授权。
4. 在 Mimi 添加 **Alibaba Cloud**，将 Key 填入「语音识别 → API Key」并保存。默认文字翻译同样使用该百炼凭证。

| 使用方式 | 当前需要的模型权限 |
| --- | --- |
| 桌面端默认识别 + 翻译 | `qwen-audio-3.0-asr-flash-streaming` 和 `qwen-mt-lite` |
| 桌面端使用独立文字翻译或跳过翻译 | 识别仍使用 `qwen-audio-3.0-asr-flash-streaming`；翻译按所选服务配置 |
| Android 默认识别 + 翻译 | `qwen-audio-3.0-asr-flash-streaming` 和 `qwen-mt-lite`（与桌面共享默认链路） |
| Android 使用独立文字翻译或不翻译 | `qwen-audio-3.0-asr-flash-streaming`；翻译按所选服务配置 |

这里需要的是**百炼 / DashScope API Key**，不要填阿里云 AccessKey ID / AccessKey Secret。内置配置使用 `dashscope.aliyuncs.com`，没有地域或端点输入框；其他地域的 Key 不能直接混用。地域对应关系见[官方地域说明](https://help.aliyun.com/zh/model-studio/regions)，共享链路及剩余平台差异见[平台差异](development/platform-parity.md)。

计费：两个平台的默认链路均包含语音识别和文字翻译两部分。更换文字翻译服务后，云端识别仍会产生用量，是否扣费取决于识别模型的试用余量和计费状态。查看[百炼模型计费](https://help.aliyun.com/zh/model-studio/model-pricing)及所用模型的试用状态。

<a id="volcano-engine"></a>

## Volcano Engine（火山引擎豆包语音）

入口：[豆包语音新版控制台](https://console.volcengine.com/speech/new/overview?projectName=default) · [API Key 管理](https://console.volcengine.com/speech/new/setting/apikeys?projectName=default) · [官方新版控制台指引](https://docs.volcengine.com/docs/DoubaoVoice/QuickStartNewConsole?lang=zh)

1. 注册或登录火山引擎，完成实名认证。
2. 在**豆包语音新版控制台**选择项目。上面的入口默认打开 `default`；使用其他项目时，请先切换过去。
3. 在「开通管理」中确认已开通**同声传译 2.0**。
4. 在**同一项目**的「API Key 管理」创建或复制 Key，回到 Mimi 的 **Volcano Engine → API Key** 粘贴并保存。

Mimi 当前只需要**一个新版豆包语音 API Key**，使用同声传译 2.0 的 `volc.service_type.10053` 资源。不要填写火山方舟的大模型 Key，也不用填写旧控制台的 AppID / Access Token 组合。服务开通、资源包和 Key 所属项目需要对应。

官方依据：[API Key 使用](https://docs.volcengine.com/docs/DoubaoVoice/APIKeyUsage?lang=zh) · [同声传译 2.0 接入](https://docs.volcengine.com/docs/DoubaoVoice/SimultaneousInterpretation20APIAccessDocumentation?lang=zh)

计费：在开通管理中查看本项目的试用余量、资源包和后付费设置。试用耗尽后可能进入后付费，不要把成功创建 Key 当作免费使用的保证。详见[计费概述](https://docs.volcengine.com/docs/DoubaoVoice/BillingOverview-15?lang=zh)。

<a id="google-gemini"></a>

## Google Gemini

入口：[Google AI Studio API Keys](https://aistudio.google.com/api-keys) · [官方 Key 指引](https://ai.google.dev/gemini-api/docs/api-key) · [Live 翻译文档](https://ai.google.dev/gemini-api/docs/live-api/live-translate)

1. 登录 Google AI Studio，按提示接受条款，并创建或选择关联的 Google Cloud 项目。
2. 在 API Keys 中为该项目创建 Key，确认该项目可以使用 Live 翻译模型。
3. 在 Mimi 添加 **Google Gemini**，填入 **API Key** 并保存。

Mimi 当前使用 `gemini-3.5-live-translate-preview`。需要 Gemini Developer API 的 Key；普通 Gemini 网页登录或 Vertex AI 的其他认证方式不能直接填入此字段。预览模型的开放范围、额度和网络可达性均以你所用项目为准。

计费：查看 AI Studio 中关联项目的用量和计费层级，以及[Gemini API 定价](https://ai.google.dev/gemini-api/docs/pricing)。不要将其他 Gemini 模型的免费额度直接套用到 Live 翻译模型。

<a id="tencent-cloud"></a>

## Tencent Cloud（腾讯云）

入口：[ASR 控制台](https://console.cloud.tencent.com/asr) · [账号信息 / AppID](https://console.cloud.tencent.com/developer) · [CAM API 密钥](https://console.cloud.tencent.com/cam/capi)

1. 登录腾讯云，按[官方开通步骤](https://cloud.tencent.com/document/product/1093/54362)完成认证并开通语音识别服务。
2. 确认账号具备**大模型实时语音翻译**的可用资源包或已开启对应后付费。
3. 在账号信息页找到 **AppID**，在 CAM API 密钥页创建密钥，并在创建时保存 **SecretID / SecretKey**；已有密钥使用此前保存的值，SecretKey 不能事后重新查询。三者使用同一账号下的凭证。
4. 在 Mimi 的 **Tencent Cloud** 配置中分别填写这三个字段并保存。

Mimi 使用 ASR 的[实时语音翻译 WebSocket 接口](https://cloud.tencent.com/document/product/1093/127565)，翻译模型是 `hunyuan-translation-lite`。AppID 不是 UIN；单个混元 API Key 不能代替 SecretID / SecretKey。此链路不需要另开普通机器翻译 TMT，也不需要为字幕开通语音合成。

计费：按核对日的[腾讯云计费说明](https://cloud.tencent.com/document/product/1093/35686)，**大模型实时语音翻译没有免费额度**，后付费默认关闭。普通实时 ASR 的免费额度不等于此服务的额度；请核对具体产品，避免只有 Key、没有可用服务。

<a id="baidu-translate"></a>

## Baidu Translate（百度智能云）

入口：[机器翻译控制台](https://console.bce.baidu.com/ai-engine/machinetranslation/overview/index) · [官方接入流程](https://ai.baidu.com/ai-doc/MT/2l317egif) · [实时语音翻译 API](https://ai.baidu.com/ai-doc/MT/Sl9p2h5k9)

1. 登录百度智能云，完成控制台要求的认证，并在机器翻译产品下创建应用或打开已有应用。
2. 确认该应用已获得**实时语音翻译**权限，并有可用的试用、资源包或付费服务。
3. 打开应用详情，复制同一应用的 **AppID** 和 **API Key**，按下表填入 Mimi 并保存。

| Mimi 字段 | 百度应用中的字段 |
| --- | --- |
| AppID | AppID |
| AppKey | **API Key** |

Mimi 的 `AppKey` 对应实时接口里的 `app_key`，官方定义为应用的 **API Key**；不要填写 Secret Key、OAuth Access Token，或另一套翻译产品的密钥。这里无需自行请求 OAuth Token。

计费：查看[实时语音翻译计费](https://ai.baidu.com/ai-doc/MT/Tl9pjqsym)。文字翻译或普通语音识别的开通状态和额度，不能代替实时语音翻译权限。

<a id="openai-realtime"></a>

## OpenAI Realtime

入口：[OpenAI API Keys](https://platform.openai.com/api-keys) · [Realtime 翻译指南](https://developers.openai.com/api/docs/guides/realtime-translation)

1. 登录 OpenAI 开发者平台，选择要使用的项目。
2. 为项目配置 API 计费，确认具备 Realtime 翻译及所需转写模型的访问权限。
3. 在 API Keys 创建项目 Key，复制到 Mimi 的 **OpenAI Realtime → API Key** 并保存。

Mimi 当前调用 `gpt-realtime-translate`，同时使用 `gpt-realtime-whisper` 获取原文。需要 OpenAI API Key，不是 ChatGPT 密码或登录令牌。**ChatGPT 订阅与 API 计费分开**；详见[官方说明](https://help.openai.com/en/articles/9039756-managing-billing-for-chatgpt-and-the-api-platform)。

计费：查看[API 定价](https://developers.openai.com/api/docs/pricing)及项目用量；创建 Key 本身不代表模型权限或 API 额度已就绪。

<a id="azure-openai"></a>

## Azure OpenAI

入口：[Azure Portal](https://portal.azure.com/) · [Microsoft Foundry](https://ai.azure.com/) · [官方 Realtime WebSocket 指南](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/realtime-audio-websockets#translate-audio-in-real-time)

1. 在可用的 Azure 订阅和支持相应模型的地域中，创建或选择 **Azure OpenAI 资源**。
2. 在该资源中部署实时翻译模型 `gpt-realtime-translate`，并按[官方实时转写说明](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/gpt-realtime-whisper)准备兼容的原文转写部署，例如 `gpt-realtime-whisper`。确认模型、地域和访问权限；找不到对应模型时，不要拿普通聊天部署名代替。
3. 记录**两个实际部署名称**，并在该资源的「Keys and Endpoint」中复制资源端点和 Key。
4. 在 Mimi 的 **Azure OpenAI** 配置中按下表填写并保存。

| Mimi 字段 | 填写内容 |
| --- | --- |
| Azure 资源端点 | 资源根地址，例如 `https://YOUR-RESOURCE.openai.azure.com` |
| 翻译部署名称 | 该资源内 `gpt-realtime-translate` 的实际部署名 |
| 转写部署名称 | 同一资源内兼容实时转写模型的实际部署名 |
| API Key | 该 Azure OpenAI 资源的 Key |

**部署名可以与模型名不同**，请复制部署列表中的名称。原文转写也必须使用实际部署名，见[微软的 Azure Realtime 专用规则](https://learn.microsoft.com/en-us/azure/foundry/openai/realtime-audio-reference)。端点只填资源根地址，不要附加 `/openai/v1` 或 API 路径。Mimi 当前接受 `*.openai.azure.com`、`*.openai.azure.cn`、`*.openai.azure.us` 资源域名；域名被接受不代表该云或地域已经开放对应模型。

Mimi 这项服务接的是 **Azure OpenAI**，不是 Azure Speech 资源。普通 OpenAI API Key、Speech Key 或 Azure 订阅 ID 都不能代替这里的资源 Key。用量计入 Azure 订阅，见[Azure OpenAI 定价](https://azure.microsoft.com/en-us/pricing/details/cognitive-services/openai-service/)。

<a id="xai-grok"></a>

## xAI Grok

入口：[xAI Console](https://console.x.ai/) · [官方快速开始](https://docs.x.ai/developers/quickstart) · [Voice API](https://docs.x.ai/developers/model-capabilities/audio/speech-to-speech)

1. 登录 xAI Console，选择或创建团队。
2. 检查团队的 API 用量、付费设置和 Voice 模型权限，再创建 API Key。
3. 在 Mimi 添加 **xAI Grok**，填入 **API Key** 并保存。

Mimi 当前使用 `grok-voice-latest`，配合 `grok-transcribe` 获取原文，属于回合式语音翻译。这里只填团队的 xAI API Key。额度和收费请查看控制台及[xAI API 定价](https://docs.x.ai/developers/pricing)，不要仅凭消费端会员状态判断 API 是否可用。

<a id="apple-speech"></a>

## Apple Speech（Mac 本地识别）

Apple Speech 只在**Apple 芯片、macOS 26 或更新版本、系统识别引擎可用**的 Mac 上提供，识别无需云服务账号或 API Key。

1. 停止字幕，添加 **Apple Speech** 配置。
2. 在「**识别语言**」中选择要使用的语言。未下载时点击旁边的「**下载并使用**」；语言包由 Apple 提供，这一步可能需要联网，下载完成后会自动设为识别语言。
3. 已下载的语言点击「**设为识别语言**」，再开始字幕。无需到 macOS 系统设置中寻找语言包入口；下载或保存失败时，错误与重试操作会留在当前语言旁。
4. 只看原文可选择「不翻译（仅原文）」；需要译文时，单独配置下方文字翻译服务。

不支持自动识别语言。设置中的语言列表显示本机支持的语种及下载状态；只有资源就绪且与当前翻译配置兼容的语言才能应用。托盘和悬浮窗使用同一已保存的识别语言，只提供已就绪且兼容的选项。Apple Speech 本身不提供文字翻译，可搭配独立的 Apple 翻译；使用远程翻译服务时，文字仍会发到该服务。平台条件见[平台差异](development/platform-parity.md)，语言参数见[语言设置](speech-language-setup.md#中文)。

<a id="custom-recognition"></a>

## 自定义语音识别

这两项供已经有兼容语音服务的用户使用，不对应另一个统一的开通网站。

1. 根据服务的官方协议，添加「**自定义识别（阿里云兼容）**」（DashScope 协议）或「**自定义识别（OpenAI 实时兼容）**」。
2. 从服务提供方获取**完整 WebSocket URL、已部署的识别模型 ID、API Key**。这三个字段在当前 Mimi 中都需要填写；文字翻译可免 Key 的规则不适用于此处。
3. 远程地址使用 `wss://`；只有本机回环地址允许 `ws://`。地址不附带用户名、密码、查询参数或片段。OpenAI 转写所需连接参数由 Mimi 添加。
4. 先开启「跳过翻译」，检查识别并试听一小段；确认能识别后，再配置独立文字翻译。

普通网页地址、HTTP 文件转写接口和 Chat Completions 接口都不能当作实时语音识别端点。具体示例、协议要求和「服务默认」语言的含义见[语音服务与语言设置](speech-language-setup.md#中文)。开通、部署费用或本机运行成本由你使用的服务决定。

<a id="text-translation"></a>

## 独立文字翻译

桌面端的 **Alibaba Cloud、Apple Speech、两种自定义识别**支持独立文字翻译。其他内置语音服务使用各自的翻译链路。Android 目前只在 Alibaba Cloud 配置中提供独立文字翻译。

先在「语音识别」部分完成配置，再在「文字翻译」选择服务。选择 DeepL、DeepLX、ChatMock 或 OpenAI 兼容接口时，Mimi 不会拿识别 Key 代替翻译凭证。默认 Alibaba Cloud 翻译已在[阿里云章节](#alibaba-cloud)说明。

<a id="apple-translation"></a>

### Apple 翻译

无需服务账号、API Key 或文字代理。仅支持符合条件、运行 macOS 26 或更新版本的 Apple 芯片 Mac；实际语言支持由系统返回。

1. 在「文字翻译」中选择 **Apple Translation** 并保存配置。
2. 选择明确的识别语言及翻译目标。若当前语言对未就绪，点击「**下载或启用语言包**」，在 Apple 界面中确认并等待完成。
3. Mimi 显示「已就绪」后，运行文字翻译检查或开始字幕。下载前退出 Apple 界面会保留未就绪状态，可以重试。

翻译资源与 Apple Speech 的识别资源分开；已有资源会复用。选择服务、检查连接和启动字幕都不会暗中下载。文字翻译在 Mac 本地执行；若识别选择的是云端服务，音频仍会发送给该服务。Android 不提供 Apple 翻译。

<a id="deepl"></a>

### DeepL

入口：[DeepL API 计划](https://www.deepl.com/en/products/api) · [账号 API Keys & Limits](https://app.deepl.com/your-account/keys) · [官方 Key 指引](https://support.deepl.com/hc/en-us/articles/360020695820-API-key-for-DeepL-API)

1. 注册适合自己的 **DeepL API** 计划，在账号的 API Keys & Limits 中创建或复制 Key。
2. 在 Mimi 的「文字翻译」选择 **DeepL**，填入完整的 **DeepL API Key** 并保存。
3. 运行文字翻译连接检查。Mimi 自动为以 `:fx` 结尾的 Key 选择 Free 端点，其余选择 Pro 端点；这里不需要填写服务地址或模型名。

找不到 Key 时先确认订阅的是 API 计划；普通翻译网页或 App 订阅不等于 API 计划。端点规则见[DeepL 鉴权说明](https://developers.deepl.com/docs/getting-started/auth)，额度与费用在账号中查看。

<a id="deeplx"></a>

### DeepLX

入口：[上游项目 DLX（原名 DeepLX）](https://github.com/OwO-Network/DLX) · [部署指引](https://deeplx.owo.network/install/) · [接口格式](https://deeplx.owo.network/endpoints/free.html)

1. 按上游说明自行部署服务，或向你选择的服务提供方获取服务地址和访问令牌。
2. 在「文字翻译」选择 **DeepLX**，填写服务地址；如果该实例要求 Bearer 令牌，填入「访问令牌」，否则留空。
3. 保存并检查翻译。Mimi 自动补上 `/translate`，也接受已经以 `/translate` 结尾的地址。

本机示例：`http://127.0.0.1:1188/translate`。远程或局域网服务器使用 HTTPS。令牌填在专用字段中，不要拼在 URL 查询参数里。DeepLX 是独立第三方服务，不是 DeepL 官方 API；不要把官方 DeepL API Key 当作实例令牌。自行部署也不代表翻译模型在本机运行，费用和可用性由实际部署及其上游决定。

<a id="chatmock"></a>

### ChatMock

入口：[ChatMock 上游项目与安装说明](https://github.com/RayBytes/ChatMock)

1. 按上游说明安装 ChatMock，在 **ChatMock 中完成账号登录**并启动服务。
2. 在 Mimi 的「文字翻译」选择 **ChatMock**；同一台电脑上的默认地址是 `http://127.0.0.1:8000/v1`。
3. 模型名称填该服务 `/v1/models` 返回的实际 ID；服务无需 Bearer 认证时，Mimi 的 API Key 留空。
4. 保存并检查翻译。其他电脑或服务器上的 ChatMock 请填写对应 HTTPS 地址。

账号登录留在 ChatMock 中，**不要把 ChatGPT 密码或登录令牌填入 Mimi**。本机地址只表示接口在本机，模型仍通过已登录账号在线调用；账号权益、额度与服务可用性按其实际状态计算。服务需返回最终文字，兼容选项可使用 `--reasoning-compat legacy --reasoning-summary none`。

<a id="openai-compatible"></a>

### OpenAI 兼容接口与 Index-Translate

向你选择的服务提供方开通文字模型，获取**服务地址、模型 ID，以及按需提供的 API Key**。在 Mimi 的「文字翻译」选择「**OpenAI 兼容接口**」，分别填写并保存。

该接口需要支持非流式 Chat Completions，并返回 `choices[0].message.content`。服务地址可填 `/v1` 等 API 基础路径，也可填完整 `/chat/completions` 路径；Mimi 只补 `/chat/completions`，**不会自动补 `/v1`**。例如：`https://example.com/v1` 会请求 `https://example.com/v1/chat/completions`。远程服务使用 HTTPS，本机回环地址可使用 HTTP；地址不接受用户名、密码、查询参数或片段。

截至核对日期，B 站 [Index-Translate 的官方示例](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py)提供以下免费、免认证的公开文字翻译接口：

| Mimi 字段 | 填写内容 |
| --- | --- |
| 服务地址 | `https://index-translate.bilibili.com/v1` |
| 模型名称 | `Index-Translate-35B-A3B` |
| API Key | 留空 |

如果同一地址以前保存过 Key，请使用「**移除翻译密钥**」再保存；单纯留空输入框可能保留旧 Key。免费接口的持续可用性以上游为准。Index-Translate 只翻译文字，云端语音识别仍可能产生费用。

<a id="original-only"></a>

### 仅显示原文

在支持独立文字翻译的配置中，开启「**跳过翻译**」即可停止发送文字翻译请求；Apple / 自定义识别也可选择「**不翻译（仅原文）**」。Android 在 Alibaba Cloud 的文字翻译选项中选择「**不翻译（仅原文）**」。

「字幕显示 → 仅原文」只改变显示方式，不等于关闭翻译请求。需要减少翻译调用时，请使用前面的跳过 / 不翻译设置。云端语音识别的用量仍会继续计算。

## Android 配置时注意

Android 的八个内置云语音服务使用上文对应凭证，但没有 Apple Speech 或两种自定义识别配置入口。[阿里云表格](#alibaba-cloud)列出当前共享模型；保存的高级设置不代表内置链路支持覆盖端点或模型。

手机里的 `127.0.0.1` 指手机自己。连接电脑上的 ChatMock / DeepLX 时，通常使用电脑的 HTTPS 服务地址；Android 的本机 HTTP 需要主动允许。模拟器和 USB 开发连接方法见 [Android 指南](../android/README.md#independent-text-translation)。

## 配好后还是不能用？

| 情况 | 先检查 |
| --- | --- |
| 鉴权失败 / 401 / 403 | 是否取错产品的 Key，是否已停用；项目、账号、地域和模型权限是否对应 |
| 有 Key，仍提示未开通或额度不足 | 开通的是不是上述具体产品；试用是否到期，资源包是否适用，后付费是否启用 |
| 找不到模型 / 部署 | 账号和地域是否开放该模型；Azure 填的是实际部署名，自定义服务填的是实际模型 ID |
| 识别有原文，没有译文 | 单独检查文字翻译；确认未开启跳过翻译、未选不翻译，并核对目标语言 |
| 保存了但没有字幕 | 运行连接检查，再播放人声并开始字幕；保存配置本身不会开始采音 |
| 某一种语言失败 | 对照[语音与语言设置](speech-language-setup.md#中文)确认具体模型与 API 的支持范围；界面可选不等于账号已获权限 |

仍有问题时，请提供 Mimi 版本、系统、服务名称、语言组合和脱敏错误信息，到 [Issues](https://github.com/yuxino/mimi/issues) 反馈。音频权限与安装问题见[使用指南](usage.zh-CN.md)。
