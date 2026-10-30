# 服务开通指南

[返回 README](../README.md) · [English](provider-setup.md) · [使用与常见问题](usage.zh-CN.md)

这份指南帮你选服务、拿到凭证并完成配置。只需开通你要用的一家。

<a id="choose"></a>

## 怎么选

| 你想怎么用 | 对应选择 |
| --- | --- |
| 已有 OpenAI 或 Gemini API 账号 | [OpenAI](#openai-realtime) 或 [Gemini](#google-gemini)。可用同声传译，也可选 ASR + MT 搭配其他文字翻译 |
| 已有 xAI API Key | [xAI 流式识别](#xai-grok)，再配置 [Grok 文字翻译](#grok-text-translation)或其他文字服务 |
| 在 Mac 本地处理 | [Apple Speech](#apple-speech)识别人声，[Apple 翻译](#apple-translation)翻译文字；需 Apple 芯片、macOS 26+ 和对应语言包 |
| 用其他服务同时完成识别和翻译 | [阿里云同声传译](#alibaba-livetranslate)、[火山引擎](#volcano-engine)、[腾讯云](#tencent-cloud)或[百度](#baidu-translate)；已有 Azure 部署可用 [Azure OpenAI](#azure-openai) |
| 需要搭配 DeepL、Grok 等文字翻译服务 | 在[阿里云](#alibaba-cloud)、[OpenAI](#openai-realtime)、[Gemini](#google-gemini) 或 [Azure](#azure-openai) 选择 ASR + MT，或使用 [xAI 流式识别](#xai-grok)，再[配置文字翻译](#text-translation) |
| 已有自己的识别接口 | [自定义识别](#custom-recognition)，文字翻译另选；必须支持相应的实时语音协议 |

语言范围和费用各不相同，按需要的语言、已有账号和预算选择。DeepL、Index-Translate 等[文字翻译服务](#text-translation)只处理文字，不能单独听声音。

阿里 LiveTranslate 配置核对日期：**2026-10-13**；其他服务说明最后核对日期：**2026-10-06**。下文以当前源码为准；旧版「语音与翻译」对应现在的「模型与服务」，实验功能可能尚未发布。Android 差异单独注明。控制台菜单、模型开放范围、试用额度和计费规则可能变化，请以对应官方文档和自己账号的状态为准。

<a id="setup"></a>

## 在 Mimi 中完成配置

1. 停止字幕，在桌面的「**设置 → 模型与服务**」或安卓的「**服务**」中添加配置。首次使用直接选择服务；已有配置时点击「添加配置」。
2. 按下文获取凭证，在对应字段分别填写并保存。凭证不需要拼成 JSON，也不要把整段示例代码粘进去。
3. 选择识别语言和翻译语言，运行配置旁的连接检查。使用独立文字翻译时，分别检查识别与翻译。
4. 播放一小段包含人声的内容，再开启「实时字幕」确认实际出字。连接检查通过只说明该次检查成功，不代表所有语言或长时间使用都已验证。

云端识别会接收所选音源的音频，远程文字翻译会接收识别文字。两路音源同时运行会分别产生服务用量。**Mimi 免费不等于服务 API 免费**：开始前查看服务是否开通、可用额度和付费方式；连接检查也可能产生少量 API 用量。不要在 Issue、日志或截图中公开真实凭证。

<a id="services"></a>

## 先找到你的服务

| Mimi 中的服务 | 开通 / 管理入口 | 需要准备 |
| --- | --- | --- |
| [Alibaba Cloud / 阿里云](#alibaba-cloud) | [北京 API Key](https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key) · [业务空间管理](https://bailian.console.aliyun.com/cn-beijing/settings/workspace) | 北京地域的百炼 API Key；[同声传译](#alibaba-livetranslate)另需对应业务空间 ID |
| [Volcano Engine / 火山引擎](#volcano-engine) | [豆包语音 API Key 管理](https://console.volcengine.com/speech/new/setting/apikeys?projectName=default) | 已开通同声传译 2.0 的同一项目内的 API Key |
| [Google Gemini](#google-gemini) | [Google AI Studio](https://aistudio.google.com/api-keys) | 可调用所选模式对应翻译或转写模型的 API Key |
| [Tencent Cloud / 腾讯云](#tencent-cloud) | [ASR 控制台](https://console.cloud.tencent.com/asr) | AppID、SecretID、SecretKey |
| [Baidu Translate / 百度](#baidu-translate) | [机器翻译控制台](https://console.bce.baidu.com/ai-engine/machinetranslation/overview/index) | 应用的 AppID、API Key（填入 Mimi 的 AppKey） |
| [OpenAI Realtime](#openai-realtime) | [OpenAI API Keys](https://platform.openai.com/api-keys) | OpenAI 项目的 API Key |
| [Azure OpenAI](#azure-openai) | [Azure Portal](https://portal.azure.com/) / [Microsoft Foundry](https://ai.azure.com/) | 资源端点、转写部署名和 API Key；同声传译另需翻译部署 |
| [xAI Grok](#xai-grok) | [xAI Console](https://console.x.ai/) | 所选团队的 xAI API Key |
| [Apple Speech](#apple-speech) | 在 Mimi 内选择语言并下载 | 符合条件的 Mac；识别无需 Key |
| [自定义识别（DashScope 协议）](#custom-recognition) | 你使用的 DashScope 兼容服务 | 完整 WebSocket URL、识别模型、API Key |
| [自定义识别（OpenAI 实时兼容）](#custom-recognition) | 你使用的 Realtime 兼容服务 | 完整 WebSocket URL、识别模型、API Key |

只想配置文字翻译？跳到 [Grok](#grok-text-translation)、[Apple 翻译](#apple-translation)、[DeepL](#deepl)、[DeepLX](#deeplx)、[ChatMock](#chatmock)、[OpenAI 兼容接口 / Index-Translate](#openai-compatible)，或[仅显示原文](#original-only)。想确认具体调用哪个模型，见[当前模型与调用方式](#current-models)。

<a id="alibaba-cloud"></a>

## 阿里云（百炼）

阿里云有两种模式：**同声传译**由同一个服务输出原文和译文；**先识别，再翻译**可搭配其他文字翻译服务。新建配置默认选同声传译，已有配置保留原模式。

入口：[北京地域 API Key 管理](https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key) · [北京业务空间管理](https://bailian.console.aliyun.com/cn-beijing/settings/workspace) · [官方开通说明](https://help.aliyun.com/zh/model-studio/faq-about-alibaba-cloud-model-studio) · [获取 API Key](https://help.aliyun.com/zh/model-studio/get-api-key)

1. 登录阿里云，按控制台要求完成实名认证，在百炼中选择**华北 2（北京）**，按提示开通该地域的模型服务。
2. 在北京地域进入 API Key 管理，使用要调用的业务空间下已有的可用 Key；不需要为每个模型单独创建 Key。
3. 如果业务空间或 Key 限制了可调用模型，确认下表中的实际模型已获授权。
4. 在 Mimi 添加 **阿里云**。使用同声模式时，按[下方步骤](#alibaba-livetranslate)填写业务空间 ID 和 Key；选择「ASR + MT」时，在「语音识别 → API Key」填入 Key。此模式的内置文字翻译使用同一百炼凭证。

| 使用方式 | 当前需要的模型权限 |
| --- | --- |
| ASR + MT | `qwen-audio-3.0-asr-flash-streaming` 和 `qwen-mt-lite` |
| ASR + MT 使用独立文字翻译或跳过翻译 | 识别仍使用 `qwen-audio-3.0-asr-flash-streaming`；翻译按所选服务配置 |
| 同声传译 | 所选北京业务空间中的 `qwen3.8-livetranslate-flash-realtime` |

这里需要的是**百炼 / DashScope API Key**，不要填阿里云 AccessKey ID / AccessKey Secret。「ASR + MT」使用 `dashscope.aliyuncs.com`，LiveTranslate 使用北京业务空间专属域名；两者均没有地域或端点输入框，其他地域的 Key 不能直接混用。地域对应关系见[官方地域说明](https://help.aliyun.com/zh/model-studio/regions)，平台差异见[平台差异（英文）](development/platform-parity.md)。

计费：语音识别和翻译都可能产生费用。更换文字翻译服务后，云端识别仍会产生用量，是否扣费取决于识别模型的试用余量和计费状态。查看[百炼模型计费](https://help.aliyun.com/zh/model-studio/model-pricing)及所用模型的试用状态。

<a id="alibaba-livetranslate"></a>

### 配置同声传译（桌面与安卓）

当前源码中的「**同声传译**」在桌面和安卓中使用共享实现，使用阿里云 LiveTranslate 模型 `qwen3.8-livetranslate-flash-realtime`，在同一音频会话中返回原文和译文，只显示文字，不播放译音。目前支持**北京地域、自动识别源语言、60 种文字翻译目标及只显示原文**。已有「ASR + MT」配置保持不变。

1. 打开[北京业务空间管理](https://bailian.console.aliyun.com/cn-beijing/settings/workspace)和 [API Key 管理](https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key)，桌面也可以在阿里设置中点击「**获取业务空间 ID**」「**获取 API Key**」。复制现有业务空间 ID 和同一空间的 Key。若看到 `llm-xxx.cn-beijing.maas.aliyuncs.com` 这样的 API Host，ID 只填 `llm-xxx`；已保存的匹配 Key 可以复用。
2. 停止字幕，桌面打开「**设置 → 模型与服务**」，安卓打开「**服务**」，添加或编辑阿里云配置，选择「**同声传译**」，填入业务空间 ID、API Key，保存并选用该配置。
3. 识别语言选「**自动识别**」，在界面可选项中选择所需翻译语言，播放一小段非敏感样本，再开启「**实时字幕**」，试完后停止。「ASR + MT」仍可选，切回后会恢复已保存的独立文字翻译服务。

若提示无权限，检查业务空间中该模型的「模型调用」权限，以及 Key 的自定义模型范围和 IP 限制。**API 调用可能收费**，开始前查看试用余量、有效期和付费状态。「原文」只隐藏译文，仍可能产生翻译用量。连接检查不代表字幕质量或长会话稳定性已经验证。

官方依据：[LiveTranslate 接入与协议](https://help.aliyun.com/zh/model-studio/qwen3-5-livetranslate-flash-realtime) · [地域与业务空间域名](https://help.aliyun.com/zh/model-studio/regions) · [业务空间与模型权限](https://help.aliyun.com/zh/model-studio/permission-management-overview) · [API Key 权限](https://help.aliyun.com/zh/model-studio/get-api-key) · [模型计费](https://help.aliyun.com/zh/model-studio/model-pricing) · [试用额度](https://help.aliyun.com/zh/model-studio/new-free-quota)

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
2. 在 API Keys 中为该项目创建 Key，确认可以使用所选模式需要的模型。
3. 在 Mimi 添加 **Google Gemini**，选择「**同声传译**」或「**ASR + MT**」，填入 **API Key** 并保存。ASR + MT 还需单独[配置文字翻译](#text-translation)。

同声传译使用 `gemini-3.5-live-translate-preview`；ASR + MT 使用 `gemini-3.5-transcribe-live` 识别，再交给所选文字服务。需要 Gemini Developer API 的 Key，不能填 Gemini 网页登录信息或 Vertex AI 凭据。模型权限、额度和网络可达性以你所用项目为准。

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
2. 为项目配置 API 计费，确认可以使用所选模式需要的模型。
3. 在 API Keys 创建项目 Key。在 Mimi 的 **OpenAI** 配置中选择「**同声传译**」或「**ASR + MT**」，填入 Key 并保存。ASR + MT 还需单独[配置文字翻译](#text-translation)。

同声传译调用 `gpt-realtime-translate`，同时使用 `gpt-realtime-whisper` 获取原文；ASR + MT 使用 `gpt-live-transcribe` 识别，再交给所选文字服务。需要 OpenAI API Key，不是 ChatGPT 密码或登录令牌。**ChatGPT 订阅与 API 计费分开**；详见[官方说明](https://help.openai.com/en/articles/9039756-managing-billing-for-chatgpt-and-the-api-platform)。

计费：查看[API 定价](https://developers.openai.com/api/docs/pricing)及项目用量；创建 Key 本身不代表模型权限或 API 额度已就绪。

<a id="azure-openai"></a>

## Azure OpenAI

入口：[Azure Portal](https://portal.azure.com/) · [Microsoft Foundry](https://ai.azure.com/) · [官方 Realtime WebSocket 指南](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/realtime-audio-websockets#translate-audio-in-real-time)

1. 在可用的 Azure 订阅和支持相应模型的地域中，创建或选择 **Azure OpenAI 资源**。
2. 先在 Mimi 选择模式。「**同声传译**」需要 `gpt-realtime-translate` 翻译部署，以及兼容的原文转写部署，如 `gpt-realtime-whisper`。「**ASR + MT**」只需要实时转写部署，[文字翻译](#text-translation)另行配置。请对照[官方转写说明](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/gpt-realtime-whisper)确认地域和模型权限，普通聊天部署不能代替实时模型。
3. 复制所需的实际部署名称，并在该资源的「Keys and Endpoint」中复制资源端点和 Key。
4. 在 Mimi 的 **Azure OpenAI** 配置中按下表填写并保存。ASR + MT 无需填写「翻译部署名称」。

| Mimi 字段 | 填写内容 |
| --- | --- |
| Azure 资源端点 | 资源根地址，例如 `https://YOUR-RESOURCE.openai.azure.com` |
| 翻译部署名称 | 同声传译时填写：该资源内 `gpt-realtime-translate` 的实际部署名 |
| 转写部署名称 | 同一资源内兼容实时转写模型的实际部署名 |
| API Key | 该 Azure OpenAI 资源的 Key |

**部署名可以与模型名不同**，请复制部署列表中的名称。原文转写也必须使用实际部署名，见[微软的 Azure Realtime 专用规则](https://learn.microsoft.com/en-us/azure/foundry/openai/realtime-audio-reference)。端点只填资源根地址，不要附加 `/openai/v1` 或 API 路径。Mimi 当前接受 `*.openai.azure.com`、`*.openai.azure.cn`、`*.openai.azure.us` 资源域名；域名被接受不代表该云或地域已经开放对应模型。

Mimi 这项服务接的是 **Azure OpenAI**，不是 Azure Speech 资源。普通 OpenAI API Key、Speech Key 或 Azure 订阅 ID 都不能代替这里的资源 Key。用量计入 Azure 订阅，见[Azure OpenAI 定价](https://azure.microsoft.com/en-us/pricing/details/cognitive-services/openai-service/)。

<a id="xai-grok"></a>

## xAI Grok

入口：[xAI Console](https://console.x.ai/) · [官方快速开始](https://docs.x.ai/developers/quickstart) · [Streaming STT](https://docs.x.ai/developers/model-capabilities/audio/speech-to-text) · [Grok Voice](https://docs.x.ai/developers/model-capabilities/audio/speech-to-speech)

1. 登录 xAI Console，选择或创建团队，确认所用 xAI 模型的 API 权限和额度，再获取 API Key。
2. 在 Mimi 添加 **xAI Grok**。新配置默认选择「**流式语音识别（ASR）**」和 **xAI Grok** 文字翻译；先在「语音识别」填写识别 Key 并保存。
3. 在「文字翻译」保留 **xAI Grok**，或选择其他支持的文字服务。使用 Grok 时，填入其 API Key、选择文字模型（默认 `grok-4.3`），再点击「**保存翻译配置**」。模型修改与其他文字服务一样，保存后才生效。
4. 识别与翻译分别保存凭据。两处可以填写同一个已有 xAI Key，也可以使用不同 Key；Mimi 不会拿识别 Key 补上缺失的翻译 Key。分别运行两项检查。
5. 不需要译文时，选择「**不翻译（仅原文）**」或开启「**跳过翻译**」；识别继续运行，不会请求文字翻译。

流式模式的识别模型是 `grok-voice-transcribe-2.0`，已确认的语音片段交给独立选择的文字服务。更换文字服务或其模型不会更换识别模型。Grok 文字翻译也可搭配[其他支持的识别服务](#text-translation)。

已有 Voice 配置保留「**Grok Voice**」，使用 `grok-voice-latest` 在讲话停顿后分段翻译。需要体验 ASR 时，手动切换语音模式，再配置文字翻译。如果曾试用未发布的旧版 ASR/Grok 组合，Mimi 会将同一配置已保存的 Key 和模型复制到独立文字设置一次；之后删除凭据不会自动恢复。

额度和收费请查看控制台及 [xAI API 定价](https://docs.x.ai/developers/pricing)，不要仅凭消费端会员状态判断 API 是否可用。

<a id="apple-speech"></a>

## Apple Speech（Mac 本地识别）

Apple Speech 只在**Apple 芯片、macOS 26 或更新版本、系统识别引擎可用**的 Mac 上提供，识别无需云服务账号或 API Key。

1. 停止字幕，添加 **Apple Speech** 配置。
2. 在「**识别语言**」中选择要使用的语言。未下载时点击旁边的「**下载并使用**」；语言包由 Apple 提供，这一步可能需要联网，下载完成后会自动设为识别语言。
3. 已下载的语言点击「**设为识别语言**」，再开始字幕。无需到 macOS 系统设置中寻找语言包入口；下载或保存失败时，错误与重试操作会留在当前语言旁。
4. 只看原文可选择「不翻译（仅原文）」；需要译文时，单独配置下方文字翻译服务。

不支持自动识别语言。设置中的语言列表显示本机支持的语种及下载状态；只有资源就绪且与当前翻译配置兼容的语言才能应用。托盘和悬浮窗使用同一已保存的识别语言，只提供已就绪且兼容的选项。Apple Speech 本身不提供文字翻译，可搭配独立的 Apple 翻译；使用远程翻译服务时，文字仍会发到该服务。平台条件见[平台差异（英文）](development/platform-parity.md)，语言参数见[语言设置](speech-language-setup.md#中文)。

<a id="custom-recognition"></a>

## 自定义语音识别

这两项供已经有兼容语音服务的用户使用，不对应另一个统一的开通网站。

1. 根据服务的官方协议，添加「**自定义识别（DashScope 协议）**」或「**自定义识别（OpenAI 实时兼容）**」。
2. 从服务提供方获取**完整 WebSocket URL、已部署的识别模型 ID、API Key**。这三个字段在当前 Mimi 中都需要填写；文字翻译可免 Key 的规则不适用于此处。
3. 远程地址使用 `wss://`；只有本机回环地址允许 `ws://`。地址不附带用户名、密码、查询参数或片段。OpenAI 转写所需连接参数由 Mimi 添加。
4. 先开启「跳过翻译」，检查识别并试听一小段；确认能识别后，再配置独立文字翻译。

普通网页地址、HTTP 文件转写接口和 Chat Completions 接口都不能当作实时语音识别端点。具体示例、协议要求和「服务默认」语言的含义见[语音服务与语言设置](speech-language-setup.md#中文)。开通、部署费用或本机运行成本由你使用的服务决定。

<a id="text-translation"></a>

## 独立文字翻译

桌面和安卓均可用**阿里云、OpenAI、Gemini、Azure OpenAI 的 ASR + MT**，或 **xAI 流式识别**，再单独选择文字翻译。桌面端还支持 **Apple Speech 和两种自定义识别**。同声传译和 Grok Voice 使用服务自身的翻译。

切换模式前先停止字幕。OpenAI、Gemini 和 Azure 在配置中选择「**翻译模式 → ASR + MT**」；阿里云选择「**ASR + MT**」，xAI 选择「**流式语音识别（ASR）**」。

先在「语音识别」部分完成配置，再在「文字翻译」选择服务。选择 Grok、DeepL、DeepLX、ChatMock 或 OpenAI 兼容接口时，分别保存对应的翻译凭据；Mimi 不会拿识别 Key 代替缺失的翻译 Key。默认阿里云翻译已在[阿里云章节](#alibaba-cloud)说明。

<a id="grok-text-translation"></a>

### Grok 文字翻译

1. 在支持独立文字翻译的配置中，在「文字翻译」选择 **xAI Grok**。可以搭配 xAI ASR，也可以搭配其他支持的识别服务。
2. 填入 xAI API Key，以及该 Key 实际可用的 Chat Completions 模型 ID；模型字段初值为 `grok-4.3`。Mimi 使用固定的 xAI 文字端点，无需填写服务地址。
3. 点击「**保存翻译配置**」，再检查文字翻译。模型和 Key 属于这个文字服务，与识别分别保存；可以使用已有 xAI Key，无需为每个模型另建 Key。

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

「**快速填写**」提供以下 13 项可编辑预设，填入服务地址及适用的示例模型；模型字段为空时，需填写实际可用的模型 ID。

选择预设后，使用附近的「**获取 API Key**」打开对应官方密钥管理页，用「**配置指南**」查看开通和模型选择步骤；帮助图标会说明当前服务的操作顺序。本机服务提供「**安装与设置**」，免 Key 服务明确显示「**无需 API Key**」。只有点击操作才打开网页，未保存的字段会保留在编辑器里。

| 服务 | 模型填写方式 |
| --- | --- |
| [OpenAI](https://developers.openai.com/api/docs/models/gpt-6-luna) | 已填示例模型，请确认 Key 已开通该模型。 |
| [DeepSeek](https://api-docs.deepseek.com/) | 已填示例模型，请确认 Key 已开通该模型。 |
| [通义千问 Qwen](https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope) | 已填示例模型，请确认地域及开通状态。 |
| [Google Gemini](https://ai.google.dev/gemini-api/docs/openai) | 填写 Key 实际可用的模型 ID。 |
| [Kimi](https://www.kimi.com/help/kimi-api/api-troubleshooting) | 填写对应地域开放平台账号可用的模型 ID。 |
| [硅基流动 SiliconFlow](https://docs.siliconflow.cn/docs/api/chat-completions-post) | 填写实际可用的模型 ID。 |
| [OpenRouter](https://openrouter.ai/docs/quickstart) | 填写包含提供商前缀的完整模型名称。 |
| [Index-Translate](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py) | 预填 `Index-Translate-35B-A3B`；公开示例无需 API Key。 |
| [腾讯 TokenHub](https://cloud.tencent.com/document/product/1823/130078) | 填写 TokenHub 已开通的模型 ID。 |
| [百度千帆](https://cloud.baidu.com/doc/qianfan-docs/s/Fm9l6ocai) | 填写千帆已开通的模型 ID。 |
| [火山方舟 / 豆包](https://docs.volcengine.com/docs/ark/base-url-and-authentication?lang=zh) | 填写方舟 Key 可用的模型或推理接入点 ID。 |
| [Ollama](https://docs.ollama.com/api/openai-compatibility) | 填写已下载的模型 ID，并启动本机服务。 |
| [LM Studio](https://lmstudio.ai/docs/developer/openai-compat) | 填写服务器已加载的模型 ID，并启动本机服务。 |

千问和火山方舟默认使用北京地址，腾讯 TokenHub 默认使用广州地址。Kimi 默认连接中国站，国际站账号可改填 `https://api.moonshot.ai/v1`；TokenHub 新加坡账号可改填 `https://tokenhub-intl.tencentmaas.com/v1`。地址须与账号的地域、API 产品和 Key 匹配。这些预设调用普通文字模型 API，不能拿腾讯语音的 AppID/SecretID/SecretKey、百度语音翻译的 AppID/AppKey 或豆包语音 Key 代替；方舟套餐订阅与普通 API 也使用不同端点。

选择预设不会保存或连接；选择另一地址的服务预设时，会清空输入中的密钥，并阻止沿用旧地址已保存的密钥。按需填写文字服务的 Key，再保存并检查翻译。本机模型服务须先启动并加载模型；Android 还需明确允许本机 HTTP。预设用于配置兼容接口，选择成功不代表账号权限、服务可用性或翻译速度已验证。

该接口需要支持非流式 Chat Completions，并返回 `choices[0].message.content`。服务地址可填 `/v1` 等 API 基础路径，也可填完整 `/chat/completions` 路径；Mimi 只补 `/chat/completions`，**不会自动补 `/v1`**。例如：`https://example.com/v1` 会请求 `https://example.com/v1/chat/completions`。远程服务使用 HTTPS，本机回环地址可使用 HTTP；地址不接受用户名、密码、查询参数或片段。

截至 2026-10-13，B 站 [Index-Translate 的官方示例](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py)提供以下免费、免认证的公开文字翻译接口：

| Mimi 字段 | 填写内容 |
| --- | --- |
| 服务地址 | `https://index-translate.bilibili.com/v1` |
| 模型名称 | `Index-Translate-35B-A3B` |
| API Key | 留空 |

保存后，运行文字翻译连接检查。

如果同一地址以前保存过 Key，请使用「**移除翻译密钥**」再保存；单纯留空输入框可能保留旧 Key。免费接口的持续可用性以上游为准。Index-Translate 只翻译文字，云端语音识别仍可能产生费用。

<a id="original-only"></a>

### 仅显示原文

在支持独立文字翻译的配置中，开启「**跳过翻译**」或选择「**不翻译（仅原文）**」，即可停止文字翻译请求。安卓可在 ASR + MT 或 xAI 流式识别配置的文字翻译选项中选择「**不翻译（仅原文）**」。

「字幕显示 → 仅原文」只改变显示方式，不等于关闭翻译请求。需要减少翻译调用时，请使用前面的跳过 / 不翻译设置。云端语音识别的用量仍会继续计算。

## Android 配置时注意

Android 的八个内置云语音服务使用上文对应凭证，但没有 Apple Speech 或两种自定义识别配置入口。[阿里云表格](#alibaba-cloud)列出当前共享模型；保存的高级设置不代表内置链路支持覆盖端点或模型。

手机里的 `127.0.0.1` 指手机自己。连接电脑上的 ChatMock / DeepLX 时，通常使用电脑的 HTTPS 服务地址；Android 的本机 HTTP 需要主动允许。模拟器和 USB 开发连接方法见 [Android 指南（英文）](../android/README.md#independent-text-translation)。

<a id="troubleshooting"></a>

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

<a id="current-models"></a>

## 当前模型与调用方式

下表列出桌面端的默认和可选调用方式。模型 ID 用于确认权限和理解用量；内置识别会自动选用模型；Azure 部署名和独立文字模型在对应字段设置。服务商可更新同名模型，模型 ID 本身不能保证识别质量或某个语言的可用性。

| 桌面服务 | 识别 / 原文与翻译所用模型或接口 | 在 Mimi 中如何选择 |
| --- | --- | --- |
| 阿里云 — 先识别，再翻译 | 识别：`qwen-audio-3.0-asr-flash-streaming`；默认文字翻译：`qwen-mt-lite` | 内置固定；可单独更换文字翻译服务 |
| 阿里云 — LiveTranslate 同声 | `qwen3.8-livetranslate-flash-realtime`；原文和译文来自同一实时会话 | 在「识别与翻译」中选择「同声传译」；见[配置步骤](#alibaba-livetranslate) |
| Google Gemini — 同声传译 | `gemini-3.5-live-translate-preview`；原文和译文来自同一 Live 会话 | 选择「同声传译」；模型内置 |
| Google Gemini — ASR + MT | 识别：`gemini-3.5-transcribe-live`；文字翻译另选 | 选择 ASR + MT 后配置文字服务 |
| Volcano Engine | 豆包同声传译 2.0，资源 ID `volc.service_type.10053`；`/api/v4/ast/v2/translate` | 内置服务接口；请求未指定公开模型 ID |
| Tencent Cloud | ASR 实时语音翻译；翻译模型 `hunyuan-translation-lite` | 内置固定；识别由该服务提供，未单独指定 ASR 模型 ID |
| Baidu Translate | 实时语音翻译 `/ws/realtime_speech_trans` | 内置服务接口；请求未指定公开模型 ID |
| OpenAI — 同声传译 | 翻译：`gpt-realtime-translate`；原文转写：`gpt-realtime-whisper` | 选择「同声传译」；模型内置 |
| OpenAI — ASR + MT | 识别：`gpt-live-transcribe`；文字翻译另选 | 选择 ASR + MT 后配置文字服务 |
| Azure OpenAI — 同声传译 | 翻译：用户部署的 `gpt-realtime-translate`；原文：用户指定的兼容实时转写部署 | 填写两个实际部署名 |
| Azure OpenAI — ASR + MT | 识别：用户的实时转写部署；文字翻译另选 | 填写资源端点、转写部署名和 Key，再配置文字服务 |
| xAI ASR | 识别：`grok-voice-transcribe-2.0`；文字翻译另选 | 选择 ASR 后配置文字服务，分别保存两阶段的 Key |
| xAI Grok Voice | `grok-voice-latest`；识别和翻译来自同一 Voice 会话 | 已有 Voice 配置保留此模式 |
| Apple Speech | macOS `SpeechTranscriber` 与已下载语言包；文字翻译单独配置 | 选择本机语言包；无云端模型 ID |
| 自定义 DashScope / OpenAI Realtime 识别 | 识别使用用户填入的模型 ID；文字翻译单独配置 | 端点、模型与识别 Key 均由用户填写 |

阿里云「ASR + MT」的内置文字翻译默认使用 `qwen-mt-lite`，不会因识别语言不同自动切换到 `qwen-mt-flash` 或 `qwen-mt-plus`。该模式的识别经 WebSocket 持续送入音频，远程独立文字翻译通过 HTTP 发送识别文字，Apple 翻译则在本地执行；更换文字翻译不会更换识别模型。

模型与路由依据：[桌面新建配置与旧配置](../src-tauri/src/settings_store.rs) · [共享服务选择](../shared/mimi-runtime/src/clients/translation_client.rs) · [当前翻译模式](../shared/mimi-runtime/src/core/configuration.rs) · [共享识别服务](../shared/mimi-runtime/src/clients/recognition_client.rs) · [各家协议实现](../shared/mimi-runtime/src/core/protocols/) · [Qwen-MT 默认模型](../shared/mimi-runtime/src/core/protocols/qwen_mt.rs)

### Android 的阿里云链路

| Android 使用方式 | 当前默认模型与调用方式 |
| --- | --- |
| 同声传译 | `qwen3.8-livetranslate-flash-realtime`；需北京业务空间 ID 和 Key |
| ASR + MT | 共享 Audio3 识别（`qwen-audio-3.0-asr-flash-streaming`）＋Qwen-MT，默认 Lite |
| ASR + MT 搭配独立文字翻译 / 不翻译 | 使用同一 Audio3 识别；启用翻译时调用所选文字服务 |

Android 正式会话与桌面使用同一个共享 Rust 服务工厂。旧 Kotlin 传输类保留用于离线回归，其中的模型不是当前正式链路的默认模型。保存的高级设置不会覆盖内置链路不支持的端点或模型。实现见[共享服务选择](../shared/mimi-runtime/src/clients/translation_client.rs)和[平台差异（英文）](development/platform-parity.md)。

### 独立文字翻译用什么模型

| 文字翻译服务 | Mimi 实际选择 |
| --- | --- |
| 阿里云分步模式（桌面 / Android） | `qwen-mt-lite` |
| xAI Grok | 默认 `grok-4.3`；在文字翻译服务内修改并保存模型 |
| Apple 翻译 | macOS Translation 框架与已下载的系统资源；无云端模型 ID |
| DeepL | 官方文本翻译 API；Mimi 不指定 `model_type` 或模型 ID |
| DeepLX | 配置地址提供的 `/translate` 服务；Mimi 不选择其上游模型 |
| ChatMock / OpenAI 兼容接口 | 使用用户填写的模型 ID，没有统一内置默认模型 |
| Index-Translate 示例 | `Index-Translate-35B-A3B`，通过 OpenAI 兼容接口配置 |
| 跳过翻译 / 不翻译 | 不调用文字翻译模型 |

请求依据：[DeepL](../src-tauri/src/core/protocols/deepl.rs) · [DeepLX](../src-tauri/src/core/protocols/deeplx.rs) · [OpenAI 兼容文字翻译](../src-tauri/src/core/protocols/openai_compatible.rs) · [Index 官方示例](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py)

配置完成后，按[启动步骤](#setup)开启字幕。录音权限、字幕样式和保存记录见[使用指南](usage.zh-CN.md)。

[返回服务列表](#services) · [返回首页](../README.md)
