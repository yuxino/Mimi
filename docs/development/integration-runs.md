# 集成实测记录

持续方法见 [集成测试与经验积累](integration-learning-loop.md)。这里记录
实测范围和未解决项；条目是历史证据，后续任务仍须核对当前源码与设备。
不写密钥、音频、字幕正文或个人目录，不把临时日志当作已持久归档的案例。

## 2026-10-04：首轮质量基线与重复尾句接纳修复

- 基线 revision：`c7e406760a915a35e0c2c7c5c963b4488c30012b`，两批
  服务测试和签名 dev 构建均为干净工作区。开发凭据沿用既有私有 loader，
  未复制密钥，未引入服务。系统声音串行播放、麦克风未使用；未发布版本。
- 冻结输入：私有 catalog `2026-10-04-semantic-8814`，12 例，五条
  FLEURS 官方 reference、Sintel 八条对白 cue；官方文本已核对，未独立
  听审。共六条不同原始真人片段、四条人工派生语音条件、两个负对照，
  不能当作十二个自然独立样本。变速 1.25 倍、混音全窗口 RMS 比 20 dB、
  重复父样本与哈希均保留；电影原声实际 SNR 和词级时钟未知。
- 执行：现有 batch runner，`qwen-audio-3.0-asr-flash-streaming`、`auto`、
  bypass；先三例，再九例，各批最多三个 worker。12 completed、0 failed。
  编译测试程序 SHA256 为
  `c2e67cd553128a24526891874e821a74cceeaec582bf68e9eaa1e252fa2c9527`。
- 直接 ASR：十个语音条件均未观察到 reference 删除。日语短句、三次
  重复、变速、混音、中文短句和电影对白在既有归一化后完全对应；日语
  长句四处替换为汉字／假名书写差异，中文长句一处为近义表达。英语和
  重复英语有专名替换及数字书写差异，错误先出现在 ASR。等值数字写法
  不算语义错误；未做总分排名，也未据此证明普遍无漏句或翻译改善。
- 负对照：音乐／音效与全零 PCM 均没有 lexical final。音乐例出现两次
  短 lexical draft，数字静音未出现；音乐源仍待独立听审。这是未解决
  观察，零 final 不等于没有瞬时识别文本，也不能据此确认模型幻觉。
- 真实 macOS 链路：固定日语 11.1 秒片段构造三次、间隔 12 秒，输入
  59.3 秒，实际播放约 60.212 秒。日语→中文、Alibaba 开发预设、系统
  全应用捕获、原文＋译文。既有字幕保存开启、音频保存关闭，开发取证
  独立显式开启并正常停止／封存。临时 UI 设置随后恢复。
- 私有原生 case `8d974297-9027-4968-8529-7d9cf10c4e6f`：1,700 条完整
  元数据、84 个快照，trace/content 丢失、失败、限额及 frontend drop
  均为零；9,291 成功发送 chunk，无失败／取消／丢弃，约 7.83 MB 音频
  证据。发送成功不等于证明服务接收。三个 source ID 2/3/4 对应 pair
  1/2/3、翻译 request 7/10/13，三条精确链均为 `server-final`。
  接纳事件 813/1133/1465；最后 history 变更 1466 后有交付快照，终态
  Idle 快照 84 的发布事件 1690，封存 1700。原文／接纳／历史均为
  138/138 字符、D/I/S 为零，三次身份独立。
- 语义：这一个父句的五个事实／条件单元在三次译文中均保留，AI review
  未见重大增漏义，有轻微直译和未译术语；没有双语人工 gold，不泛化为
  其他十例 MT 合格。原生正文窗口未独立确认；store applied／DOM commit
  及滚动溢出观察不是逐字可见性证明。
- 实际执行边界：原计划暂停／恢复未能按时操作，因此三个 occurrence
  均计入采集，没有排除区间；清空未执行。停止发生在 finals 已完成后，
  不能作为待译尾句竞争、in-flight clear、暂停恢复或重连的验收。
- 确认修复：桌面 HQ `FinalRequestKey` 的 finish 通配条件忽略原句身份，
  在同文旧句 active／queued 时跳过另一句停止尾部，最早失效为 final
  接纳。通过真实 `flush_pending_draft` 的回归复现旧版失败；补入原句 ID
  与 content revision 后通过。无 ID 的合法 ASR 路径以不同本地确认编号
  保留独立任务，同一已知原句版本的 server/finish alias 仍去重。没有改
  提示词、模型、队列上限、重试、预算或 grace。见
  [设计记录](../plans/2026-10-04-final-request-identity.md)。
- 相邻回归：59 项 HQ 测试通过，覆盖 active／queued 的已知和无 ID 尾句、
  同源 alias、内容版本、preview 抢占、HTTP 顺序／取消／预算和重试。
- 自动检查：`./scripts/check.sh` 全部通过，桌面 Rust 1010 passed／
  2 ignored；前端 95 个文件、1113 项测试通过。共享 Rust 65 单元测试、
  3 契约测试，JNI crate 格式／Clippy／编译测试边界均通过；这不等于
  Android JVM 实际 JNI 或设备实测，Android CI 另行核对。
- 修复签名构建：干净 `ca759df` 通过 canonical dev launcher，稳定 dev
  requirement 与既有身份相同。两次 normal 启动的设置页超时，线程栈
  确认全配置目录的凭据状态检查等待 `SecKeychainFindGenericPassword`／
  SecurityServer，其余读取等待 secret cache 锁。活动 dev 预设不能隔离
  目录中普通配置的 Keychain 检查；未把超时归因于本次尾句代码。
  工具禁止操作 SecurityAgent，未绕过授权。改用 `--ui-only` 后设置与
  诊断正常载入，Idle、追踪关闭，重开原 case 得到 1700 元数据／84 快照／
  零证据缺失。正常退出，原配置及沉浸设置恢复；无凭据 UI 和离线回放
  不等于修复后的真实 provider 验收。
- 独立只读审阅未发现此次身份匹配引入的错误去重或重复接纳路径。
- PR 首次 CI：macOS、Windows x64／ARM64、前端及 Android 检查通过，
  Linux 普通 Rust／HQ／共享检查通过，但既有双输入 PulseAudio smoke 在
  `system_pipeline.finish(1s)` 失败。源码显示测试模拟发送器使用 32 槽
  sink，却先等待 finish 再消费 sink；满队列时无法完成发送。日志未记
  当时队列占用，不能把这次具体超时直接定性为队列满或负载波动。
  测试 harness 改为先匹配两路时持续读取双方，停止一条时继续读活跃
  另一条，finish 时并发消费 stopped sink。新增满 32 槽加一个 pending
  帧的确定性单元回归，要求全部 33 帧收尾、零遗留、旧 ingress 关闭。
  保留原 1 秒 finish、6 秒双频率界限、两种采样率和分离／重启断言；
  生产音频行为、队列容量与 deadline 未改。该 Linux 模块的实际测试
  由 Linux CI 验证，macOS 完整检查不冒充 Linux 原生证明。
- 持久保存：私有结果 catalog `2026-10-04-quality-cycle-1`，约 22.3 MB、
  142 条文件 SHA256，12 个 relocated result 路径和 PCM hash 已复核。
  原始 job 文件和失败日志保留，固定矩阵指向持久输入库；新录制 case
  仍在 app 私有 workspace `quality-cycle-1`。没有把音频、字幕或个人
  路径放入 Git。

### 下一轮的固定优先级

1. 同一输入／配置下专测 Stop 紧接未完成 final，保留新的 clean revision
   case；提前确认实际原生控件，分别测试暂停恢复、in-flight clear 和
   重连。不能用这次成功的 server finals 替代 finish-race 复测。
2. 独立听审音乐负对照，并检查瞬时草稿到 stop fallback 的条件；不添加
   基于几个词的过滤规则。
3. 以既有语义单元逐例补日／英→中文、中文→英语的准确 MT 请求／返回。
   优先英语专名与日语长句指代，区分 ASR 继承错误、reference 歧义和 MT
   新增错误。自然多分钟语音、重叠及 code switching 仍缺样本。
4. 保留旧失败证据，不把一次成功或 provider 波动当成修复改善。Windows、
   Linux、Android 真实设备与实际 provider 账户仍需各自验收。

## 2026-10-04：最新 main 的 macOS dev 集成检查

- Revision：`092a31008e54c4cc2f5033f43eee700b97a51e59`；fetch 后
  `HEAD == origin/main`，构建时工作区干净。包含桌面与 Android 共享核心。
- 构建与启动：使用 `./scripts/dev-app.sh`，复用已有缓存。
  `/Applications/mimi-dev.app` 的稳定签名、
  `app.yuxino.mimi.dev` 身份、单一准确进程路径和 `(dev)` 窗口已确认。
  正式版通过正常退出让出运行；本轮未发布版本。
- 范围：macOS、本机系统声音、全部应用捕获、Alibaba 开发预设、
  日语识别到简体中文、原文＋译文。既有字幕保存开启，音频录制关闭；
  使用不含敏感内容的合成测试语音，未开启开发音频与字幕取证。
- 自动检查：`./scripts/check.sh` 全部通过。桌面 Rust 1008 passed／
  2 ignored；前端 95 个文件、1113 个测试通过；共享核心 65 个单元测试
  与 3 个契约测试通过，并完成 JNI crate 的格式、Clippy 和编译检查。
  这里的 JNI crate 检查不等于 Android JVM 实际 JNI 测试或设备验收。
- 服务连接：识别检查 346 ms、翻译检查 388 ms，均可用；这些是连接／
  固定短句检查的计时边界。
- 真实链路：有效日语样本约 11.12 秒，确认实际播放，观察到识别和中文
  译文，原生浮窗显示双方文本。内容无关追踪观察到服务 final、最终翻译、
  快照发布及 overlay 收到、应用和 DOM commit。原生可见性另由窗口检查确认。
  浮窗显示最近接口往返 43 ms、翻译请求耗时 333 ms，未测端到端延迟。

### 可复用经验

- macOS 沙箱内 `say -o` 可返回零却生成零音频帧；`afinfo` 暴露了空文件。
  `afplay` 在沙箱内也曾返回 `AudioQueueStart failed (-66680)`。
  使用经批准的本机执行重新生成后，核对非零帧与时长，再执行播放。
  音源失败时先修正测试准备，不能归因于 Mimi 无字幕。
- 设置页、字幕控制窗口和字幕正文窗口是不同原生窗口。确认当前窗口
  标题；关闭设置页后检查实际字幕正文及其控件，不能拿设置预览替代它。
- 稳定 dev 身份、真实服务可用和原生字幕可见分别验证，结果分别记录。
  复用有效缓存完成构建，不把本次构建时间当成通用性能提升。

### 未完成与下一步

- 用户中断操作时，dev 应用和系统声音字幕会话仍在运行。暂停、恢复、
  停止及尾部收尾尚未测试；没有把它们计入通过范围。
- 只跑了一个短合成样本，译文有一处测试语境的用词偏差。语义质量、
  真人长语音、音乐背景、重叠说话、清空、折叠和重连没有完成验收。
- 本次输入与日志位于临时目录，尚未归档为可持久复用的场景案例。
  普通追踪只有有界内存尾部，未建立可重启回放的完整取证案例。
- Android JVM／设备、Windows 和 Linux 没有在本轮实测。
- 后续获得继续测试的请求后，先补会话暂停／恢复／停止与收尾，再按当前
  具体质量问题选定可持久样本，记录 reference 并进行可比复测。

## 2026-10-04：Gemini 连接拒绝与真人音频直连复测

- Revision：`a2cf738` + `fix/gemini-live-setup` 未提交改动；macOS 原生
  `GeminiLiveClient` 测试构建，开启 `local-dev-credentials`。使用用户授权的
  免费 Gemini 测试账户、本机私有开发文件和固定 16 kHz 单声道 PCM，
  按 100 ms 实时发送。未打开 GUI、未启动系统／麦克风捕获，未修改
  保存选项或活动配置。密钥、音频、reference、原始服务正文和配对结果
  均在 Git 外的持久开发 benchmark 目录；普通输出只有计数、标签和计时。
- 连接失败边界：同一有效密钥列模型 HTTP 200；旧 setup WebSocket 1007，
  `inputAudioTranscription` 层级被拒绝。两项转录选项移到 setup 后接收
  `setupComplete`，原生 Settings 检查可用，971 ms。共享 setup 契约覆盖
  zh/en/ja；不改模型、翻译配置或提示词。
- 首次真人测试还复现语言字段单独出现时解析失败、连续服务无
  `turnComplete` 导致尾部不确认。最初逐句修复在重复音频中串句并
  `gemini_close_timeout`：服务译文与原文的标点数不能证明语义配对。
  保留旧失败记录；改用共享 Rust 的完整块稳定检查点，2 秒无非空增量
  且双方均结束于句末标点后整块确认。实际 JNI 与桌面使用同一逻辑；
  显式 turn 仍留 500 ms 吸收尾部。稳定检查点是启发式，不是服务终止保证。
- 固定 FLEURS 真人样本（3 个独立输入、1 个重复派生输入，CC BY 4.0）：

  | 样本／方向 | 音频时长 | setup | 首条译文预览 | 完整块确认 | 错误 |
  | --- | --- | --- | --- | --- | --- |
  | `fleurs-english-1527` 英→中 | 23.44 s | 961 ms | 3,769 ms | 25,545 ms，1 组 | 0 |
  | `fleurs-cmn_hans_cn-short-row3-id1648` 中→英 | 9.66 s | 928 ms | 5,928 ms | 11,045 ms，1 组 | 0 |
  | `fleurs-ja_jp-short-row0-id1519` 日→中 | 11.10 s | 830 ms | 5,035 ms | 13,099 ms，1 组 | 0 |
  | `repeat-en-two` 英→中，两遍同输入间隔 1 s | 47.88 s | 962 ms | 3,816 ms | 25,491／50,739 ms，2 组 | 0 |

  首条译文和块确认均从开始送音频计时，含样本开头静音和服务处理；
  不是从首个发声或每个词到屏幕的端到端延迟。两遍相同原文均保留，
  无串句及收尾超时。取证文件 0600，旧失败与原始事件记录继续保留。
- 质量：链路可用，但三组均有准确性问题。英→中保留主要赛事／名次，
  姓名识别错误，译文另把一个运动员扩成两个姓名。中→英将 reference
  地名识别成别处；另一次同输入连谓语也错。日→中将回国后的时间条件
  识别为策划后的条件，译文转录也出现该错误。该模型直接做语音翻译；
  不能据转录共同错误断定内部采用 ASR 文本再翻译。reference 来自既有固定 FLEURS
  corpus，尚无本轮独立听审；不据此给整体准确率或宣称优于其他服务。
- 回归保护：共享 setup／转录序列、实际 Android JNI、桌面 socket 模拟
  覆盖缺省空文本、译文拆句、重复、显式边界晚尾、未配对文本与固定
  内存上限。`./scripts/check.sh` 通过（桌面 1,015、前端 1,116）；
  开发特性 Clippy 通过，Gemini 定向测试 25 passed／2 ignored；Android
  `testDebugUnitTest lintDebug` 通过，包含实际 JNI。最终原生连接检查
  1,005 ms 可用。签名 dev 已更新到 `/Applications/mimi-dev.app`，稳定
  designated requirement 相同，`app.yuxino.mimi.dev`；正常退出旧实例后
  `--no-launch` 安装，未打开新实例。此构建仍是 dirty 测试产物。
- 模型调研：官方账号模型清单含实际请求的专用
  `gemini-3.5-live-translate-preview`（`3.5-live-translate-06-2026`）；
  Gemini 路由直接取原文／译文转录，没有独立二次 MT。官方模型能力和
  参数依据见本轮 design note。用同一中文 PCM 比较两种额外配置：
  专用模型请求 TEXT 仍生成 816,000 字节音频、无 model text parts，
  有英文译文转录；3.8 Live 沿用 translationConfig 接受 setup，却用
  中文对话回应，未执行目标英语翻译。连接成功不能证明字段生效或
  模型可以互换。只做转录正文比较，未保存或听审输出音频。
- 免费档位：官方把同一专用翻译模型列为 Free／Paid 可用；额度和
  数据政策存在档位差异，但没有发现准确度降级声明。本轮没有付费
  同模型对照，不能把错误归因于免费账号，也不能证明两档效果相同。
- 限制／下一步：未验证原生浮窗、OS 捕获、Android 实机、跨平台真实
  服务和长时连续无停顿语音。完整块策略可能延迟确认，持续输入到达
  5,120 字符上限会有界失败；短样本成功不能替代长会话验收。先按相同
  reference 人工听审与逐语义单元对比，再决定是否继续使用此 preview
  模型；不要用词汇替换补丁掩盖服务 ASR／翻译错误。
