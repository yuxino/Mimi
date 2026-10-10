# Speech services and language setup

[English README](../README_EN.md) · [中文 README](../README.md) · [English](#english) · [中文](#中文)

## English

A third-party service can use a language selected in Mimi **only if its endpoint, protocol and model accept the parameter Mimi sends**. Choosing a language does not add it to a model. Speech recognition and text translation are separate stages.

### Configure a custom recognition service

1. Open **Settings → Speech & Translation**, add **Custom Recognition (DashScope Compatible)** or **Custom Recognition (OpenAI Realtime Compatible)** according to the service's documented protocol.
2. Enter its **full WebSocket URL**, deployed **recognition model ID**, and API key. A website, HTTP file-transcription endpoint, or Chat Completions endpoint cannot be used as live recognition.
3. Save recognition settings and select the profile. Optionally declare the recognition languages documented by that deployment; leave the range undeclared if unknown. The declaration is saved with this profile and filters the language picker, but does not query or change the model. An empty declaration leaves only Service default. Enable **Skip translation** to test recognition alone. The recognition language below belongs to this active profile.
4. Choose **Service default** to omit language parameters. Use a specific language only when the service documents that code and parameter. Saving only stores the configuration. A successful recognition check confirms the setup exchange, not recognition quality or support for every displayed language.
5. Use **Check recognition** beside recognition. Once recognition works, configure and check text translation separately, then disable Skip translation if needed.

Examples of configuration values (use your own service key; region/account availability still applies):

| Protocol | Full WebSocket address | Recognition model example | Specific English choice sends |
| --- | --- | --- | --- |
| DashScope | `wss://dashscope.aliyuncs.com/api-ws/v1/inference` | `qwen-audio-3.0-asr-flash-streaming` | `payload.parameters.language_hints: ["en"]` |
| OpenAI standalone transcription | `wss://api.openai.com/v1/realtime` | `gpt-4o-mini-transcribe` | `session.audio.input.transcription.language: "en"` |
| OpenAI live transcription family | Same standalone transcription URL, or a documented compatible endpoint | `gpt-live-transcribe` | `session.audio.input.transcription.languages: ["en"]` |

The model name selects the field shape; it does not establish compatibility of a custom host. Remote addresses require `wss://`; only loopback accepts `ws://`. No URL query parameters are accepted by the custom editor. Bearer authentication, PCM format, acknowledgements and result events must match the selected protocol. See [DashScope events](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-client-events) and [OpenAI transcription](https://developers.openai.com/api/docs/guides/realtime-transcription).

The custom picker exposes Mimi’s complete encodable registry. This is **not a model support list**. A profile’s declaration narrows these choices. Independent translation further intersects the recognition and text service catalogs. Original-only retains the recognition catalog. OpenAI-compatible/ChatMock targets remain configurable instructions whose support depends on the selected model.

### Official API audit — 2026-10-06

These are documentation and request-contract checks, not live acceptance tests for every language. Settings, tray and floating subtitle controls share the same capability resolver and saved source/target selection. Tray and floating controls also expose the target picker for integrated services such as Gemini. Native target switching rejects an unsupported direction before saving rather than substituting another target; a listening session reconnects, while a paused session stays paused.

| Current service/model | Automatic input and language choices | Primary evidence |
| --- | --- | --- |
| Desktop Alibaba Audio3 + Qwen-MT Lite | Auto omits the hint; 30 explicit ASR hints, 31 text targets plus Original. Translation intersects both catalogs. | [Audio3 client events](https://help.aliyun.com/zh/model-studio/qwen-audio-asr-streaming-client-events), [Qwen-MT model tables](https://help.aliyun.com/zh/model-studio/machine-translation#supported-languages) |
| Android Alibaba `qwen3.5-livetranslate-flash-realtime` / `qwen3-asr-flash-realtime` | Auto already existed. Built-in route now exposes 60 languages; independent text uses 27 explicit ASR hints plus Auto. `nb`/`fil` wire aliases remain model-specific. | [Live translation](https://help.aliyun.com/zh/model-studio/qwen3-5-livetranslate-flash-realtime), [ASR events](https://help.aliyun.com/zh/model-studio/qwen-asr-realtime-client-events) |
| OpenAI `gpt-realtime-translate` | Automatic input, all 13 documented targets already present. | [Dedicated model language table](https://developers.openai.com/cookbook/examples/voice_solutions/realtime_translation_guide#supported-languages) |
| Azure deployment of `gpt-realtime-translate` | Automatic input; remove Mimi’s three-target guard and use the model’s 13 targets. Microsoft directly demonstrates German; the full catalog follows the same model’s publisher documentation. This is a model-identity inference, not live Azure proof. | [Microsoft model identity](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/gpt-realtime-translate), [Microsoft translation example](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/realtime-audio-websockets#translate-audio-in-real-time), OpenAI table above |
| Google `gemini-3.5-live-translate-preview` | Automatic input already present; expand 30 to 78 targets. Preserve `zh→zh-Hans`, `zh_tw→zh-Hant`, `tl→fil`, and explicit `pt-BR`/`pt-PT`. Generic Live transcription hints do not establish fixed-source support for this model. | [Model language table](https://ai.google.dev/gemini-api/docs/live-api/live-translate#supported-languages), [generic transcription field](https://ai.google.dev/api/live#AudioTranscriptionConfig) |
| xAI `grok-voice-latest` | Auto omits `language_hint`. Expose all 20 documented codes for explicit hints and prompt-directed targets. Arabic/Spanish/Portuguese regions remain distinct; a hint is not a strict filter. | [Speech-to-speech languages and hints](https://docs.x.ai/developers/model-capabilities/audio/speech-to-speech#supported-languages) |
| Tencent `speech_translate` + `hunyuan-translation-lite` | Nine explicit sources with a target matrix. No universal Auto. `zh_en` is Chinese/English mixed mode; its same-code target means bidirectional translation. | [Exact speech translation API](https://cloud.tencent.com/document/product/1093/127565) |
| Baidu `realtime_speech_trans` | All 45 source/target languages with the vendor’s exact codes, such as `ja→jp` and `fr→fra`. No general Auto field is documented for this API. | [Exact real-time speech translation API](https://ai.baidu.com/ai-doc/MT/Sl9p2h5k9) |
| DeepL text | Expand the local adapter to 114 source and 120 target codes/aliases. Automatic source omits `source_lang`. Recognition choices still depend on the selected ASR. No account language lookup is made. | [Language API documentation](https://developers.deepl.com/docs/getting-started/supported-languages), [official SDK types](https://github.com/DeepLcom/deepl-node/blob/main/src/types.ts), [request contract](https://developers.deepl.com/api-reference/translate/request-translation) |
| DeepLX text | Separate 32 source / 38 target choices including aliases and regional outputs; automatic source uses `auto`. Actual support depends on the deployed compatible server version. | [Upstream supported language table](https://deeplx.owo.network/endpoints/free.html#supported-languages) |
| Apple Speech / Translation | Read current system languages and readiness. Explicit source required; recognition and translation packs are separate. New regional choices require exact native support. | [SpeechTranscriber](https://developer.apple.com/documentation/speech/speechtranscriber), [Translation](https://developer.apple.com/documentation/translation) |
| Volcano AST2.0 S2T | Expand to 20 ordinary languages and input-only Cantonese/Shanghainese. Source or target must be Chinese/English. Explicit Chinese ↔ English reversal sends `zhen` on both sides; it is not universal Auto. Canonical `yue`/`wuu` map to the vendor's `yue-CN`/`sh-CN`. | [Exact AST2.0 API, language sets and constraints](https://docs.volcengine.com/docs/DoubaoVoice/SimultaneousInterpretation20APIAccessDocumentation?lang=zh) |

The checked automatic modes were already exposed. No universal Auto option is invented for an explicit-input API. Catalogs use the integrated model’s documented scope rather than another model from the same company. Unsupported language pairs fail before connection; changing a source keeps the target when legal and otherwise chooses a supported fallback.

Shared wire fixtures are consumed by desktop Rust and Android tests. The encodable registry and provider catalogs are checked separately so adding one service’s language cannot silently broaden another. Custom recognition editors and declarations remain desktop features; Android’s differing Alibaba models remain explicit platform differences. No paid cloud audio request was made for this audit.

## 中文

**三方服务可以配置语言，前提是该地址、协议和识别模型支持 Mimi 发出的参数。** 选语言不能给模型增加语种；识别语言与翻译目标是两回事。

### 怎么接入

1. 打开「设置 → 语音与翻译」，按服务文档选择「自定义识别（阿里云兼容）」或「自定义识别（OpenAI 实时兼容）」。
2. 填入完整 WebSocket 地址、实际部署的识别模型 ID 和密钥，点击「保存识别配置」。不能填网页地址、HTTP 文件转写接口或 Chat Completions 地址。上方表格给出了官方地址与模型示例；不要把翻译模型填进识别模型。
3. 可按该部署的文档声明它支持的识别语言；不确定时保持未声明。声明只保存在这项配置中并筛选语言选项，不会查询或改变模型；声明为空时只保留「服务默认」。使用这项服务配置，先开启「跳过翻译」，只测试语音转文字，再在下方选择「识别语言」。
4. 「服务默认」代表不发送语言参数，是否自动检测取决于对端。选择英语等具体语言会发送对应参数；请先确认该模型接受这个代码。保存只写入配置；识别检查成功仅说明本次建立会话成功，不证明识别质量或所有候选语言均可用。
5. 点击识别旁的检查操作。识别正常后，再独立配置并检查文字翻译，按需关闭「跳过翻译」。

阿里兼容接入发送 `language_hints: ["en"]`；OpenAI 独立转写通常发送 `transcription.language: "en"`，`gpt-live-transcribe` 系列改用 `transcription.languages: ["en"]`。自动/服务默认时省略这些字段。模型名只用于选择字段形状，不能证明任意兼容地址支持该能力。区域地址与账户权限请以服务自己的文档为准；这里不会自动探测私有地址。

自定义识别提供 Mimi 可编码的完整语言目录，**不是任意模型的支持承诺**。配置里声明的语言范围和独立文字翻译服务会进一步筛选；只显示原文时保留识别服务自己的范围。通用 Chat Completions／ChatMock 的目标是传给模型的翻译指令，实际效果仍由模型决定。

### 本次核对与修复

上方表格逐项列出了当前模型、自动识别语义、修复范围及官方出处：

- Gemini 从 30 补到 78 个目标；葡萄牙语地区变体单列。OpenAI 保持完整的 13 个，Azure 同名专用模型从 3 个补到 13 个。
- xAI 补齐 20 个官方语言／地区代码。自动识别省略提示；阿拉伯语、西班牙语和葡萄牙语的地区不擅自代选。
- 百度补齐 45 个语言及其专用代码。腾讯补齐 9 个识别选项，并按识别语言筛选目标；「中英混合」不等于通用自动识别，同模式互译也不会被当成无需翻译。
- DeepL 与 DeepLX 分别按自己的目录扩充；计数含地区和兼容别名。自动文字检测保留，识别语言仍与所选 ASR 求交集，不会让阿里 Audio3 凭空支持更多语言。
- Android 阿里内置同传和独立识别使用不同模型，分别处理 60 个同传语言与 27 个显式识别提示。桌面 Audio3 的 30 个提示及 Qwen-MT Lite 的 31 个目标保持原模型范围。
- Apple 根据系统实际目录及就绪状态显示，区域语言需要准确匹配。设置、托盘与悬浮字幕的来源和目标选择共用相同规则及已保存状态；Gemini 等一体服务也可在托盘和悬浮窗选择翻译目标。原生切换会在保存前拒绝不支持的方向，不会擅自换成另一种目标；运行中的会话重连，暂停状态保持暂停。

核对到的通用自动识别模式原本均已有入口；没有给要求明确输入的接口虚构「自动」选项。火山补齐 20 种普通语言、粤语与上海话输入，以及「中英互译」模式；识别或翻译一侧必须为中英，方言仅作输入。中英互译同时发送 `zhen`，不等于任意语种自动识别。Azure 的完整 13 项依据相同模型身份与模型发布方文档推导，尚未逐项调用 Azure 验证。

这些结论来自官方资料、实际请求编码及桌面／Android 共用合成用例，不代表每种语言的云端账号、语音质量或完整字幕会话均已实测。本轮语言目录审计没有发送云端付费音频。Apple 语言包下载与文字检查的原生实测另记于集成记录。
