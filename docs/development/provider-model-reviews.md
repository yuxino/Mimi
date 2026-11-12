# 供应商与模型巡检记录

维护方法见[供应商与翻译模型维护](provider-model-maintenance.md)。
每轮记录核对日期、源码、官方来源、范围、结论及下一步；实际模型对比结果
另入[集成实测记录](integration-runs.md)。本页不保存媒体、译文或账户信息。

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
