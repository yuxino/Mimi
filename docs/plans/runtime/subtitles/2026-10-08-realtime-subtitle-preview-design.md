# 实时字幕越过上一完整句

离线真实 Kotlin/JNI 回放已确认：完整 A 后，B 原文和译文草稿进入共享核心，
Android 却继续读取 A 的 `displayPair`，直到完整 B 才整句替换。这证明显示
等待存在，尚不证明 #211 无声录屏的全部延迟都来自显示层。

在共享核心新增可替换的 `realtimePreview`，与已确认的完整 `displayPair`
和历史分开。实时事件推进预览；上一完整句仍在核心中保留。新原文先显示
单行，译文到达且句子身份相同后显示两行；不能把上一句的 final 或身份
不明的译文与带有身份的当前原文组合。无 ID 事件保留既有兼容语义。
重复措辞仍按身份区分。迟到 final 可以写入
历史并替换其完整对，不能倒退更新的实时预览。清空、重连释放预览。

Android 只读取共享投影，不增加 Kotlin reducer。`displayPairFinal` 描述
实际展示的完整确认；草稿不触发已完成字幕的滚动定位。独立 ASR + HTTP
文字翻译仍通过 `SourceUtteranceDraft` / `PreviewPair` / `ConfirmedPair`
保留完整对。桌面的现有实时路径也消费共享投影。当前桌面新会话通过
`effective_translation_mode` 统一为 Turbo，阿里云与独立文字翻译继续采用
完整预览对；关闭 interim 显示时同样沿用完整对策略。

共享契约覆盖 A→B 草稿→B final、译文先到、身份不匹配、同文不同句、
迟到结果、历史开关、清空/重连和独立 HTTP。直接 Rust、stateless bridge、
实际 JNI 和桌面投影分别验证。没有新增语音调用或捕获；设备实际呈现和
端到端延迟需要同一 APK、片段及无正文时间戳另行验证。


## 运行链路共用

用户要求安卓与 PC 保持同一套代码逻辑。因此修复范围包含实际运行代码，
不能只共用 reducer 或复制 PC 策略为 Kotlin。原 desktop provider clients、
协议、AudioSendPipeline、HighQualityTranslationClient、配置/能力归一化和
TranslationSessionController 迁至 shared/mimi-runtime；desktop 原模块仅
re-export。Android MimiService 使用 SharedRuntimeEngine 的 JNI 入口，直接
接收同一 Controller 的 snapshot。它不再创建 DashScopeEngine、
StreamingServiceEngine 或 Kotlin TranslationPipeline。独立文字连接检查也
调用 PC 的 connection_diagnostics，未使用真实账号执行检查。

两端使用共享 RecoverySchedule、MTBudgetContinuity、错误分类、健康探测和
pending 期限。Android 仅适配 MediaProjection、系统代理和默认平台信任根；
不将密钥、任意 provider 错误内容或请求正文写入日志。活动句柄与连接检查
句柄分别有上限，停止删除句柄、取消工作及健康探测；PCM/事件仍使用 PC 的
有界队列。正常结束保留共享允许的可靠尾部确认。

设置使用同一 Rust 语言能力与 Lite/Flash/Plus 配置，默认 Lite。旧自定义
model/base URL/hotword 值保留在存储，但内置服务的界面不再呈现不生效选项。
Android 本地明文连接还需经过系统 NetworkSecurityPolicy；模拟器宿主
10.0.2.2 是平台限定的回环例外，PC 回环规则不放宽。

本地测试证明共享事件处理与实际 JNI/PCM 模拟连接工作。APK 编译和离线测试
不等于设备端长期网络/TLS/采音验收，也不证明原录屏二进制与源码精确匹配。
最小运行验证需同一构建的 APK hash，以及 PCM 入队、识别草稿到达、MT 起止、
snapshot 发布与界面呈现的无正文时间戳；本次未新增录音或付费调用。


展示策略也由实际共享 factory 选择：Audio3 + MT 显示成对预览，集成实时服务
显示所属当前句的可替换草稿。Controller 发布 atomicPreview 元数据，PC 和
Android 都使用它；原文模式继续展示识别行。两端快照刷新使用同一 60 ms
策略，Android 没有把每条原文草稿强行展示为与 PC 不同的单行新句。
