# 音频质量基线重放

本页是 [持续集成测试流程](integration-learning-loop.md) 的下一轮操作入口。
使用已有 runner、analyzer 与私有样本，不另建执行器；“强化”指改进验证和
回归保护。已执行的 revision、数量和限制见 [实测记录](integration-runs.md)。

## 先重开归档，避免从头收集

从实测记录定位 Git 外的私有 catalog，读取目录清单、输入 manifest、
`complete-service-runs-index.json`、播放收据和对应原生案例索引。索引不是
服务重放指令，也不是正确答案。逐条确认引用文件仍存在于持久目录，重开
PCM、request、events、metrics，核对保存的 hash；搬迁后的临时绝对路径
必须修正并重新校验，不能只改汇总数字。失败、未执行与成功记录都保留。

| 已有输入 | 下一轮用途 | Reference 与使用边界 |
| --- | --- | --- |
| FLEURS | 固定日／英／中朗读对照 | 固定数据版本与 CC BY 4.0；官方 reference 尚需独立听审 |
| ASCEND | 固定中／英／混用对话对照 | 固定数据版本与 CC BY-SA 4.0；定向片段不代表全部自然对话 |
| 官方动画预告 | 背景音乐、角色轮换的私有链路探针 | 版权素材、未知 gold；自动视频字幕和服务输出不充当 reference |

复用归档中的 FLEURS／ASCEND 矩阵。合并矩阵时保留原 label、来源、许可证、
父样本、hash、语言、单位与 `expectedSpeech`，核对各输入的绝对路径；不要
重新选片替换曾经失败的样本。重复输入另有运行身份，不能冒充新独立样本。
私有目录 0700、文件 0600；不将媒体、正文、密钥或个人路径复制进 Git。

## 先准备，再决定本轮执行范围

在仓库根目录使用已校验的私有矩阵：

```bash
python3 -B scripts/run-development-batch.py \
  --manifest /absolute/private/next-cycle-corpus-matrix.json \
  --output-root /private/tmp/mimi-debug-benchmark/replay-preparation
```

没有 `--run` 时只验证并写入新的 `batch-*` 私有准备目录，不读取凭据、
不编译，也不调用服务。保留 `matrix.json`、`scene.json`、输入 hash 与
`progress.json`；`prepared` 不是服务成功。准备目录仍是临时产物，若要
下次继续复用，完成后需持久归档并验证索引能重新打开。

当前 runner 要求单声道 PCM16／16 kHz、WAV 与 PCM 完全一致，每批最多
32 例。语音例必须有非空 reference；无对白／数字静音例使用
`expectedSpeech:false`、`referencePath:null`。未知 reference 的动画不能
伪装成无语音对照，也不能将版权来源标成 CC 来通过许可证 gate。保持
既有私有手动探针方式，单独记录来源、使用依据、输入与未知 gold 状态。

付费执行需要当次范围支持，才添加 `--run` 和明确的 `--jobs`（1–4，默认
3）。已有样本、历史许可或本页不会自动扩大付费数量。复用已确认的
`CARGO_HOME`、`CARGO_TARGET_DIR`、共享核心缓存与前端缓存；先核对并发使用，
不清空有效缓存。按 [开发凭据规则](local-dev-credentials.md) 读取开发版当前选中的普通服务配置及本地凭据文件，环境变量仅用于非秘密的路径／模式，不传密钥。

## 每个原生案例先证明输入路径

1. 协调唯一 `/Applications/mimi-dev.app` 的安装和原生控制时段。确认
   实际进程、稳定签名、bundle identifier、clean/dirty revision、非
   UI-only 模式与当前 provider；其他任务在测试中替换 app 会使归属失效。
2. 使用本轮指定的系统声音、捕获范围和语言，原生播放串行。播放前检查
   源文件时长、格式、hash，并听审是否有预期语音。语音例的源 PCM 应
   非零；有意的数字静音对照允许全零，不能被这一检查误判为坏样本。
3. 浏览器元素的 `muted:false`、`volume:1` 和非零元素音轨不证明系统
   已输出声音；另核对 tab／Space 静音、系统输出与实际 sent WAV。
   改用本机播放器会改变输出路径，记录变化，不能据后来成功认定旧失败
   的唯一原因。全零 sent PCM 的语音例先排查输入，不评分为 ASR 漏识别。
4. 明确启用私有音频＋字幕取证，确认新的 active case ID 和 tracing
   状态，再启动会话与播放。每次操作前获取最新 AX 树，按当前名称和
   role 定位控件；不复用旧编号。播放前确认停止控件可达，定位失败记录
   为实际失败，不把计划中的停止时间当成已执行。
5. 分别记录播放开始／结束、会话开始／停止、取证停止与 seal 状态。
   播放约 30 秒不表示捕获只有 30 秒；sent WAV 总长包含前后静音和
   控制等待。暂停、清空等有意排除区间也需记录，延迟对比使用相同边界。
6. 正常停止会话、等待尾部完成并封存取证，再开始下一例。检查 send、
   content、trace、frontend flush 的失败／取消／丢失／上限；没有完整
   seal 或证据缺失的案例保持未知。退出后恢复临时设置与配置，不改
   用户最新开发凭据，不覆盖正式安装；保留可复用缓存与旧失败证据。

## 分层比较，使用准确身份关联

| 层 | 需要检查的证据 | 不能据此单独宣称 |
| --- | --- | --- |
| 源与 sent 音频 | 源 hash、播放区间、sent WAV／index、非零区间和发送结果 | 本地 socket send 成功不证明服务已消费音频 |
| 直接 ASR 与原生 ASR | 实际 request／model／language、draft、final ID、task finish | final 数量不等于角色数或对白覆盖率；直接 PCM 与系统重采集路径不同 |
| 独立 MT 与后端接纳 | 原文、request／result、accepted pair 的准确 join | 文字相同或串行 worker 不证明因果身份；链路完整不证明译文准确 |
| 确认历史 | 同一来源与代次的 pair ID、快照和容量状态 | 历史条数不等于所有服务输出已显示 |
| 原生显示 | 对应窗口收到／应用、DOM commit、实际正文窗口及滚动状态 | DOM commit 或 overflow 计数不证明全文可读或丢字 |

准确关联至少包含 source、generation、content revision、source utterance ID、
pair ID 与 request ID，引用 trace event ID、private event 行和 snapshot ID。
缺失或歧义不靠文本补齐。区分 `server-final` 与停止时的 `session-finish`
草稿 fallback；后者不能计为服务确认 final。先定位最早有证据的偏差，再
决定修改哪一层，避免为单个专名或词语加过滤、替换和提示词补丁。

有 reference 的 corpus 可沿用离线 analyzer：

```bash
python3 -B scripts/analyze-development-case.py \
  --case /absolute/private/native-case \
  --media-manifest /absolute/private/media-manifest.json \
  --clip stable-scene-label \
  --asr-result /absolute/private/baseline-batch \
  --output-directory /absolute/private/new-analysis
```

`--media-manifest` 使用 analyzer 的 `clips` schema，不直接把 runner 的
`scenes` 矩阵传入。`--asr-result` 接受目录：可以是含 `events.jsonl` 的
单例服务目录，或含 `baseline-index.json` 的批次目录；后者要求 `clips`
中恰好一条 `baseLabel`／`label` 匹配，且 `eventsPath` 有效。不要直接传
索引文件。catalog 完整索引若文件名或结构不同，从条目选取实际单例服务
目录。输出目录使用新的私有目录；报告包含正文，只留在本机。

动画 gold 未知时先重开原始事件与身份链，不伪造空 reference 计算 CER。
离线字词差异也只对已声明的 reference 有效；固定归一化版本，不把不同
工具的全角数字、标点或分词规则变化称为质量提升。独立听审和双语语义
单元评审才能支持准确度判断。

## 下一轮优先解决的三项缺口

1. **独立听审与语义评审。** 先重听已有日语动画和异常长英语样本，记录
   时段、可听原文、存疑处与听审者；校订 reference 另存版本与 hash，
   不覆盖旧参考。单独评审专名、命题增删、角色轮换和重叠语音，MT 对照
   从听审原文建立。未经听审的差异保留为候选，不定性为幻觉。
2. **长字幕实际可见性。** 在真实正文浮窗检查长句、折叠／展开、历史
   滚动与稳定文本，保存对应快照／事件身份及原生观察；设置预览和 DOM
   commit 不能代替此项。先确认历史完整，再判定投影或窗口是否有问题。
3. **未完成 final 紧接 Stop。** 沿用固定输入，记录 Stop 时的未完成请求、
   后续 final 边界与代次，检查尾部接纳、最终历史、terminal publication、
   frontend flush 和封存。已有正常 server final 成功案例不能替代这一
   race；clear／重连按问题需要另列，不能默认扩大成新批量服务测试。

每轮只选择当前问题需要的样本和操作。私有 catalog 留完整索引、输入与
构建／播放／分析收据；公开实测记录只写 revision、模式、实际执行范围、
计时边界、失败／未知及下一检查。源码／CI 通过、原生测试通过和用户
体验通过分别记录，不能把开发验收升级为正式发布或其他平台验收。
