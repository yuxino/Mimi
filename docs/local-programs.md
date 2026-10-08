# Own models and speech services / 自带模型与语音服务

## 使用自己的模型

在「语音与翻译 → 添加配置 → 自定义 → 使用自己的模型」创建配置：

1. 选择识别引擎。whisper.cpp 使用官方 `whisper-cli` 可执行文件与 GGML `.bin` 模型；选择与你的操作系统和处理器匹配的程序。
2. 分别选择程序和模型文件，保存配置，再点击「检查运行」。检查会实际加载模型执行一次静音识别；不会下载模型或采集系统声音。
3. 使用此配置，选择语言，启动实时字幕。只看原文不需要密钥，文字翻译另行配置；云端翻译会接收识别文字。

whisper.cpp 按短句识别，句末显示，每段最长 8 秒。CLI 每段重新加载模型，适合先试用自己的模型；较大模型可能更慢，持续流式识别可使用兼容程序。`.en` 模型仅适合英语；其他语言请使用相应多语模型，具体能力取决于模型。启动参数每行一个，无需引号；允许线程／处理器／beam／best-of 数值以及 GPU／flash-attention／fallback 调节。模型、音频、语言和输出选项由 Mimi 填写。普通 Python 脚本、任意模型格式、whisper-server 不能当作 whisper-cli 使用。

模型和程序由用户自己管理。Mimi 只在检查或字幕会话时运行程序，停止时结束自有进程；每段临时 WAV 为私有文件，在完成、失败或取消后清除。删除配置不会删除这些自带文件。内置下载模型仍在单独的「本地模型」入口管理。

## 连接已有服务

选择「连接已有服务」，按服务提供的实时协议选择 **OpenAI Realtime** 或 **DashScope**，填写完整 WebSocket 地址与模型名，再保存和检查。HTTP 文件转写接口（如 `/v1/audio/transcriptions`）不属于这两个实时协议。本机 `localhost`、`127.0.0.1`、`[::1]` 服务未配置密钥时可留空；远程服务须使用 `wss://` 并填写密钥。改变地址后不会自动向新地址发送旧密钥。

## Mimi-compatible workers

Choose **Mimi-compatible worker**, then an executable, a model file/folder, and any arguments (one argument per line, no shell quoting). `{model}` and `{language}` expand within a single argument. This adapter is for an explicit worker implementing the following UTF-8 JSON-lines protocol, not arbitrary command-line tools:

- Emit `{"type":"ready"}` after loading the model. Use stdout only for protocol events; stderr is discarded.
- Accept `start` with integer `id` and `revision`, then `audio` with base64 PCM16 little-endian mono at 16 kHz, and `commit`. Each turn contains at most 8 seconds.
- Emit replaceable `draft` and one durable `final`, each carrying that turn’s `id`, `revision` and `text`; optional `language` is a Mimi source-language code. Each line is bounded to 64 KiB and transcript text to 32 KiB. Empty finals complete a turn without adding a subtitle.
- Accept `clear` (discard current content), `ping` with integer `id` (reply `{"type":"pong","id":...}`), and `finish` (emit `{"type":"finished"}` and exit). Stale revisions are discarded by Mimi.
- Emit `{"type":"error"}` on failure. Mimi displays a sanitized label, never the worker’s private output.

These desktop adapters share the existing subtitle and independent translation pipeline on macOS, Windows and Linux. Bundled MLX/Qwen downloads require Apple silicon; native acceptance on one platform does not certify the others.

Official CLI contract: [whisper.cpp CLI](https://github.com/ggml-org/whisper.cpp/tree/master/examples/cli). Worker reference: [bundled local helper](../src-tauri/local-speech/Sources/MimiLocalSpeech/main.swift).
