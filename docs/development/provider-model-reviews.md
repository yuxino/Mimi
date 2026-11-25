# 供应商与模型巡检记录

维护方法见[供应商与翻译模型维护](provider-model-maintenance.md)。
每轮记录核对日期、源码、官方来源、范围、结论及下一步；实际模型对比结果
另入[集成实测记录](integration-runs.md)。本页不保存媒体、译文或账户信息。

## 2026-11-26 · v1.7.9 发布前复核

从 `v1.7.8` 到 `7b1286a3e`，共享运行时仅包含下方 `gemini-2.5-flash` 思考参数兼容修复及测试模块外置重构；默认模型、服务文案与语言目录无其他变动。本轮沿用当日对 `f7f300d1` 的 Gemini 实测记录及 2026-11-23 的其余供应商官方来源核对结论，未新增额外供应商调用。

## 2026-11-26 · Gemini 12 月 1 日重定向实测与参数兼容修复

基于 `f7f300d1` 按用户授权只读使用开发版已保存 Gemini 凭据核对官方目录与线上接口，不保存译文或凭据：

- `/v1beta/models` 仍提供内置 `models/gemini-3.5-live-translate-preview` 与 `models/gemini-3.5-transcribe-live`（`bidiGenerateContent`）。公开样本 `en-tail-16000` 的 `manual_provider_public_audio_probe` 全程通过（连接 995 ms，首段与恢复语音各确认 1 组字幕，`session_finished: 1`）。本机正式版与开发版已保存配置均未使用 `gemini-2.5-flash` 或 `gemini-2.5-pro`。
- 在 `generativelanguage.googleapis.com/v1beta/openai/chat/completions` 上，无历史调用记录的账户请求 `gemini-2.5-flash`、`gemini-2.5-flash-lite`、`gemini-2.5-pro` 已返回 `HTTP 404`。Google 将于 12 月 1 日把存量 `gemini-2.5-flash` 请求转到 `gemini-3.5-flash-lite`：实测 `gemini-3.5-flash-lite` 在非流式与 SSE 流式下均以 `HTTP 400 INVALID_ARGUMENT` 拒绝 `reasoning_effort: "none"`，省略该参数时返回 `HTTP 200`（`prompt_tokens: 24, completion_tokens: 9, total_tokens: 33`，短句翻译默认无额外思考开销）。
- 从 `translation_thinking.rs` 移除 `gemini-2.5-flash` 及其已退役 2025 预览快照，避免 12 月 1 日自动路由后触发 `HTTP 400`；保留未参与重定向的 `gemini-2.5-flash-lite`。`gemini-2.5-pro` 本就不附加思考参数；其重定向目标 `gemini-3.8-flash` 实测以 `HTTP 400` 拒绝 `reasoning_effort: "minimal"`，无参请求在当次窗口返回临时 `HTTP 503 UNAVAILABLE`，质量与费用对照留待负载恢复后与既有候选一并评测。[Google 模型生命周期及迁移说明](https://aistudio.google.com/docs/deprecations) · [OpenAI 兼容层参数](https://ai.google.dev/gemini-api/docs/openai)

## 2026-11-25 · v1.7.8 已保存会话批量删除发布前复核

从 `v1.7.7` 到 `68efe8bca`，共享运行时仅在 `session_history` 新增批量删除能力；语音识别与文字翻译路由、默认模型、服务文案及语言目录均无变动。本轮沿用 2026-11-23 的官方生命周期与候选模型核对结论，未新增供应商调用或质量实测。

## 2026-11-23 · v1.7.7 桌面交互修复发布前复核

从 `v1.7.6` 到 `ab47ef3e`，共享运行时、服务文案及语言目录没有变动。本轮仅核对官方文档。

- Google 将于 12 月 1 日把 `gemini-2.5-flash` 的请求转到 `gemini-3.5-flash-lite`，把 `gemini-2.5-pro` 转到 `gemini-3.8-flash`，并按新模型计费。使用这两个自填文字模型的配置需在该日前核对兼容性和费用。Mimi 的 Gemini 文字预设没有预填模型，内置 3.5 同传、识别均未公布关闭日期。[Google 模型生命周期及迁移说明](https://aistudio.google.com/docs/deprecations)
- OpenAI 和 Azure 的实时语音退役表、xAI 和 DeepSeek 的滚动别名与 11 月 21 日的记录一致。腾讯轻量／标准和 Qwen-MT 的候选均仍在官方目录；继续按 11 月 21 日的顺序做同样本对照，未新增质量或时延实测。[OpenAI](https://developers.openai.com/api/docs/deprecations) · [Azure](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-retirement-schedule?view=foundry) · [xAI](https://docs.x.ai/developers/release-notes) · [DeepSeek](https://api-docs.deepseek.com/updates/) · [腾讯](https://cloud.tencent.com/document/product/1093/127565) · [Qwen-MT](https://help.aliyun.com/zh/model-studio/machine-translation)

## 2026-11-21 · v1.7.6 桌面悬停读数

OpenAI 同传的输出字幕按增量事件返回。桌面可以据已接收的当前译稿显示进度。Mimi 的这条路线已有接口往返计时，未提供逐句翻译耗时样本。此次修复只消费已有会话状态。沿用当日其他供应商的核对记录，模型及调用方式保持原配置。[实时翻译事件](https://developers.openai.com/api/docs/guides/realtime-translation) · [当前模型](https://developers.openai.com/api/docs/models/gpt-realtime-translate)

## 2026-11-21 — 豆包同传与 11 月 20 日到期项

从 11 月 18 日的 `06f51c27` 到 `c134bd3b`，共享运行时和服务预设没有变化。本轮核对公开文档，未调用供应商或读取账户配置。

- 豆包同传走 AST v2 的 `/api/v4/ast/v2/translate`，请求 `s2t` 和固定资源 `volc.service_type.10053`。原文与译文各有开始、更新、结束事件。11 月 13 日的升级指引适用于 RTC 应用，Mimi 直接连接 AST。服务端权重版本未知。[豆包同传协议](https://docs.volcengine.com/docs/doubao-voice/simultaneous-interpretation2-0-api-access-documentation?lang=zh) · [RTC 升级范围](https://docs.volcengine.com/docs/real-time-communication/real-time-call-subtitles-and-translation?lang=zh)
- Azure 将 `gpt-realtime-translate` 和 `gpt-realtime-whisper` 的 **2026-05-06** 版本列为 **2027-11-09** 退役。Mimi 使用配置中的部署名，实际版本需由部署方确认。OpenAI 直连现用的两个专用模型未列入 2027 年 1、2 月的退役批次。Gemini 3.5 专用同传和识别仍未公布关闭日期。[Azure 退役表](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-retirement-schedule?view=foundry) · [OpenAI 弃用表](https://developers.openai.com/api/docs/deprecations) · [Gemini 生命周期](https://aistudio.google.com/docs/deprecations)
- DeepL 已明确省略 `model_type` 时按 `latency_optimized` 目标路由，与 Mimi 的请求一致。具体模型由服务按语言对和请求选择，显式质量优先的收益仍待比较。xAI 的 `grok-voice-latest` 指向 3.0，独立识别已用 2.0。DeepSeek 上次公布的 `deepseek-flash` 版本为 V4.1-Flash-1017。[DeepL 选型规则](https://developers.deepl.com/docs/translate/understanding-model-types) · [xAI 更新](https://docs.x.ai/developers/release-notes) · [DeepSeek 更新](https://api-docs.deepseek.com/updates/)
- 保留腾讯轻量版和 Qwen-MT Lite。下次同样本评测先比较腾讯轻量／标准版，再补 Qwen-MT 三档的术语样本、阿里 ASR 3.0／3.1 以及文字预设 `qwen-flash`／`qwen3.8-flash` 的长尾失败。账户可用性、实付费用、其余兼容预设和本地模型仍未覆盖，完整巡检继续待办。[腾讯翻译模型](https://cloud.tencent.com/document/product/1093/127565) · [Qwen-MT 选择](https://help.aliyun.com/zh/model-studio/machine-translation) · [阿里模型上架记录](https://help.aliyun.com/zh/model-studio/newly-released-models)

## 2026-11-18：v1.7.4 发布定向复核

- 基于 `06f51c27`，保留当前模型与已保存配置。OpenAI 的[模型目录](https://developers.openai.com/api/docs/models)
  仍列专用 `gpt-realtime-translate`／`gpt-realtime-whisper`；[弃用表](https://developers.openai.com/api/docs/deprecations)
  的旧实时模型 2027-01-20、旧文件转写模型 2027-02-26 期限不能套用成这两条
  专用路线已经停用。未进行新模型质量比较。
- [Gemini 生命周期表](https://aistudio.google.com/docs/deprecations)对现用 3.5
  专用翻译／识别尚未公布停用日期。[xAI 公告](https://docs.x.ai/developers/release-notes)
  的 transcribe 1.0 已于 2026-10-02 退役，当前代码使用 2.0；voice latest
  已指向 3.0，保留既有滚动别名的质量未知边界，不将 API 兼容声明当作字幕质量实测。
- [腾讯现用接口](https://cloud.tencent.com/document/product/1093/127565)仍列
  speech_translate 和 hunyuan-translation-lite，未见下线声明；DeepL 的[旧鉴权弃用](https://developers.deepl.com/docs/resources/breaking-changes-change-notices/november-2025-deprecation-of-legacy-auth-methods)
  不影响当前 Authorization header 路径。[阿里上新](https://help.aliyun.com/en/model-studio/newly-released-models)
  的通用思考模型不直接替代专用字幕协议或低延迟文字路线。
- 所查来源未发现现用路线的立即退役阻断；这不是账号可用性或全部供应商验收。
  未调用供应商、读凭据或执行付费比较。此前候选、未覆盖项目和 2026-11-20
  复查安排保留，完整巡检仍未完成；发布范围与实测结果见集成记录。

## 2026-11-16：OpenAI 实时翻译错误核对

- 路线仍为 `gpt-realtime-translate`，源转写为 `gpt-realtime-whisper`。
  [翻译事件](https://developers.openai.com/api/reference/resources/realtime/translation-server-events)
  的错误含结构化类型与可选错误码。按
  [官方错误指南](https://developers.openai.com/api/docs/guides/error-codes)
  区分访问拒绝、额度和请求限流；无确切额度码的 HTTP 429 不推断欠费。
  本次只调整恢复与反馈，模型、音频协议和服务配置保持原样。

## 2026-11-14 — 二进制 JSON 与识别握手边界

- 核对 [Gemini WebSocket schema](https://aistudio.google.com/api-reference/live) 与
  [专用识别文档](https://aistudio.google.com/docs/live-api/live-transcribe)：
  `voiceActivity` 是服务端独立元数据，不应被识别客户端当成损坏转录。
  保留现有模型与参数；这是协议兼容修复，不是新模型的线上质量结论。
- 横向检查既有 JSON Binary 通路，移除 Audio3、LiveTranslate、OpenAI/Azure
  Realtime 和 Gemini Live 的宽松 UTF-8 替换；非法输入沿用协议失败路径。
  Qwen-MT 非流式 HTTP 响应也移除宽松替换，保留原有受限重试语义。
  腾讯/百度/xAI 的二进制音频及火山 protobuf 不适用此 JSON 修复。
- 验证契约与剩余分类边界见[设计记录](../plans/runtime/transport/2026-11-14-provider-binary-json.md)。
  本次未新增真实账号/音频测试，不替代下方未完成的全量模型复核。

## 2026-11-13 — 正式凭据复测与 Terra 参数修复

- 在 `300fd7c2` 基线上补齐 `gpt-6.1-terra` 的既有关闭思考策略：官方
  OpenAI 使用 `reasoning_effort: none`，OpenRouter 使用既有 reasoning
  开关；未知模型后缀及网关不附加参数。没有替换默认或已保存模型。
  [Terra 官方参数](https://developers.openai.com/api/docs/models/gpt-6.1-terra)
  支持该值；[Sol](https://developers.openai.com/api/docs/models/gpt-6.1-sol)
  不支持，仍不列入。共享 wire fixture 同时覆盖直接 Rust 与实际 JNI。
- 用户授权只读使用正式安装包的本地凭据。实际 JNI 生产文字连接检查中，
  Terra 请求成功（3,058 ms）；这证明本次账户和参数兼容，不是语义质量比较。
- 正式 Key 的 OpenAI、百度、阿里 ASR＋Qwen、阿里 ASR＋DeepL 完整 JNI
  会话均通过固定公开样本的「语音 → 90 秒静音 → 语音 → 收尾」。百度观测
  到三次恢复；其他三条未观测到恢复。Gemini、火山、腾讯、阿里 LiveTranslate
  另由直接生产客户端通过相同短回归。开发 Key 的 OpenAI 启动拒绝未在正式
  Key 下重现，不能认定原因；直接客户端的 OpenAI、Qwen、DeepL 传输失败
  仍保留。完整方法和局限见[集成实测记录](integration-runs.md)。
- 本轮修复已证实的协议遗漏，不据一次成功上调质量结论。上一轮 Qwen、
  ASR、DeepL 的候选决策及未覆盖路线仍有效，完整巡检尚未完成。

## 2026-11-13 — 发布前候选实测与补充核对

- 源码基线 `e50ff30746c96971c6a9fa03a5e62c7361c10e44`，产品代码未修改。
  本轮补查实际生效路线，并用现有开发凭据完成有界服务回归和候选比较；
  方法、数量和失败见[集成实测记录](integration-runs.md#2026-11-13供应商回归与模型候选比较)。
  没有改写已保存模型或新配置默认值。
- **完整巡检仍未完成**：未覆盖所有兼容预设、账户、语言、价格与弃用公告，
  也没有原生捕获／显示验收。以下结论只适用于记录中的样本。
  待办复查日期仍为 **2026-11-20**。

| 路线 | 当前证据与决策 | 下一步 |
| --- | --- | --- |
| 阿里 ASR 3.0 → 3.1 | **保留当前，候选待扩测**。[官方目录](https://help.aliyun.com/en/model-studio/newly-released-models)已列 `qwen-audio-3.1-asr-flash-streaming`。既有探针在共享域名完成一例日语 3.1，另一例英语在发送 PCM 前网络失败；可比的无上下文 3.0 日语参考编辑距离为 0，3.1 为 2，不能据版本或单例延迟宣称提升 | 保留失败，补充同配置、多语言和真人语义检查；核对[3.1 参数](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-client-events)及[workspace 连接建议](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-websocket-api)，不把网络失败误判为模型不可用 |
| Qwen 文字预设 `qwen-flash` → `qwen3.8-flash` | **保留当前**。[旧别名仍等同 2025-07-28 快照](https://help.aliyun.com/zh/model-studio/qwen-flash)，[3.8 确为可用候选](https://help.aliyun.com/en/model-studio/qwen3-8-flash)。相同六组英／日文本、同提示和关闭思考：旧版 6/6 成功，3.8 为 5/6；成功请求完成中位数 759.5/1164 ms，3.8 另有 8009 ms 的未分类请求失败。局部语义改善不足以抵消本轮不稳定性 | 复核失败类别、长尾与更多真实字幕样本；本轮是独立 HTTP 探针，采用前仍需生产客户端和原生链路验证 |
| Qwen-MT Lite／Flash／Plus | **保留 Lite 默认**。生产客户端 36/36 完成；Flash 有局部口语自然度收益，也出现技术术语错义，Plus 未显示稳定优势。三档已经可选，不属于缺失模型支持 | 扩充经人工审核的真实对白和技术上下文，保持模型各自既有协议提示，不由小样本改全局默认 |
| DeepL `model_type` | **保留当前，质量目标候选待复测**。[官方说明](https://developers.deepl.com/docs/translate/understanding-model-types)确认省略参数默认 `latency_optimized`。显式两种目标的 12 次请求全部成功，响应报告各自目标；六组中五组译文相同，尚无稳定质量收益 | 目标并非固定模型 ID；补充有差异的语言／样本，再决定是否增加用户选项，不能称现有服务一直使用旧模型 |
| OpenAI 文字候选 | **待实测**。当前预设仍为 `gpt-6-luna`；[Terra 6.1](https://developers.openai.com/api/docs/models/gpt-6.1-terra)支持关闭推理，但当前 `translation_thinking.rs` allowlist 未包含它。[Sol 6.1](https://developers.openai.com/api/docs/models/gpt-6.1-sol)不能套用同一关闭参数 | 明确推理条件和费用后再比较；本轮未测这些文字模型，不能将语音连接拒绝归因于文字候选 |
| 已更新的专用语音路线 | **已核对文档，部分已回归**。阿里实际一体式路线已用 [3.8 LiveTranslate](https://help.aliyun.com/en/model-studio/livetranslate-realtime-websocket-access)，不是遗留 3.5 常量；OpenAI 的[专用识别](https://developers.openai.com/api/docs/guides/realtime-transcription)与[专用翻译](https://developers.openai.com/api/docs/guides/realtime-translation)仍是现有模型路线 | 阿里短回归通过；OpenAI 当前会话在连接阶段被拒绝，根因仍未知。通用对话模型不能直接代入专用协议 |

腾讯轻量／标准版质量对照、Azure 实际部署版本、Gemini 文字候选及其余兼容
预设仍是待办。Gemini 文字预设本身不填写模型，不能据此声称存在过期默认；
[最新模型和生命周期](https://aistudio.google.com/docs/deprecations)与专用 Live
翻译协议分开核对。百度当前专用接口与[官方文档](https://ai.baidu.com/ai-doc/MT/Sl9p2h5k9)
一致，未发现可仅换模型名的直接升级；本轮静音后断开仍需定位。

这些结果支持继续有选择地评测，**不支持整体模型升级或大版本质量验收通过**。
保留默认不意味着所有候选更差；实际费用、真实字幕质量和跨平台原生行为仍需补证。

## 2026-11-13 — 首轮官方文档核对

- 源码基线：`e10b3ad50c82d50c576aeee2f3ca97265e2f9e0f`。
  核对共享配置、实际 factory、协议常量和
  [文字服务预设](../../shared/provider-copy.json)，不是用户当前保存的配置。
- 范围：以下路线的有界桌面研究。没有调用供应商、比较译文、测量延迟或
  费用，也没有更换模型；“待实测”不代表已经证明新模型更好。
- 完整巡检状态：尚未完成；本表不重置完整巡检日期。
  下一次供应商／发布任务先处理下列待办；文档复查日期为 **2026-11-20**，
  到期未执行时保留为逾期待办。本记录不创建定时任务或追加付费授权。

| 路线与当前实现 | 官方证据与状态 | 下一步 |
| --- | --- | --- |
| 腾讯实时翻译：固定 `hunyuan-translation-lite` | **待实测**。[同一 API](https://cloud.tencent.com/document/product/1093/127565)同时支持 `hunyuan-translation` 标准版 | 优先比较轻量／标准版的相同音频、翻译质量、延迟与计费；通过后再决定是否增加选择或改变新配置默认 |
| 阿里 ASR + MT：Lite 默认，Flash／Plus 已可选；legacy HighQuality 用 Plus | **待实测**。[官方 MT 文档](https://help.aliyun.com/zh/model-studio/machine-translation)仍列三者，定位与流式行为不同 | 用同一识别结果重比三者，评估默认是否合适；不是补做已有的模型选择实现 |
| Qwen 兼容文字预设：`qwen-flash` | **待实测**。[当前模型页](https://help.aliyun.com/zh/model-studio/qwen-flash)说明其等同 `qwen-flash-2025-07-28`；[上架记录](https://help.aliyun.com/zh/model-studio/newly-released-models)列出 2026-08-26 的 `qwen3.8-flash` | 评测新候选的字幕翻译与请求参数兼容性；与专用 Qwen-MT 分开 |
| DeepSeek 兼容文字预设：`deepseek-flash` | **已核对文档，待实测**。[2026-10-17 更新](https://api-docs.deepseek.com/updates/)将该别名指向 V4.1-Flash-1017 | 重新建立同样本基线；代码不变也可能发生模型漂移 |
| xAI 一体式：`grok-voice-latest`；识别路线：`grok-voice-transcribe-2.0` | **已核对文档，待实测**。[Voice 文档](https://docs.x.ai/developers/model-capabilities/audio/speech-to-speech)将别名指向 3.0；[STT 文档](https://docs.x.ai/developers/model-capabilities/audio/speech-to-text)列当前 2.0 | 复测别名更新后的字幕表现，区分语音翻译与独立识别路线 |
| Gemini 专用翻译：`gemini-3.5-live-translate-preview`；识别：`gemini-3.5-transcribe-live` | **已核对文档**。[翻译](https://aistudio.google.com/docs/live-api/live-translate)与[识别](https://aistudio.google.com/docs/live-api/live-transcribe)文档仍使用这些 ID；[弃用表](https://aistudio.google.com/docs/deprecations)本轮未公布两者关闭日期 | 继续跟踪预览模型生命周期；更高版本对话模型不是同协议的直接替换 |
| OpenAI 兼容文字预设：`gpt-6-luna` | **待实测**。[当前模型目录](https://developers.openai.com/api/docs/models)仍列 Luna，也列 6.1 Sol／Terra 等候选；[Sol 参数](https://developers.openai.com/api/docs/models/gpt-6.1-sol)不支持关闭推理 | 先核对实际请求参数与思考策略，再选少量候选比较翻译；不能照搬当前低延迟设置或仅换模型名 |
| DeepL：官方 v2 请求未指定 `model_type` | **未知／待实测**。[接口文档](https://developers.deepl.com/api-reference/translate/request-translation)提供质量／延迟模型选项；本轮未确认服务为该配置实际选用哪一种 | 先核实默认选择及账户／语言限制，再比较显式质量选择；不能据省略参数推断旧模型 |
| 火山同传：v2 translate 接口及资源 `volc.service_type.10053` | **已核对文档**。[官方接口](https://docs.volcengine.com/docs/doubao-voice/simultaneous-interpretation2-0-api-access-documentation?lang=zh)与当前 endpoint／资源一致 | 跟进同一服务后端变化；接口相同不能证明模型及翻译质量未变 |

### 未覆盖与后续顺序

本轮没有完成 Azure 部署版本、OpenAI 一体式与识别模型、阿里一体式及 Audio3、
百度、Apple、本地服务、自定义模型、DeepLX 和其余兼容预设的上游生命周期核对；
也未完成各服务价格、账户可用性与全量弃用公告检查，状态均为**未知**。
后续完整巡检按实际 factory 与共享预设逐项补齐，不能将本轮结果泛化到这些路线。

先准备腾讯轻量／标准版、已有 Qwen-MT 三种选择、Qwen 文字预设候选的同样本矩阵，
然后在有界授权内执行；同时补查未覆盖路线和别名变化。一次只改变模型或待比较参数，
不与提示词、分句、上下文策略重写混在一起。每项以实测结果决定采用、保留或继续待办。
