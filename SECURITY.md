# 安全策略 / Security Policy

## 报告漏洞 / Reporting a vulnerability

请不要公开提交可能泄露凭证、绕过权限或执行任意代码的安全问题。请通过 GitHub 仓库所有者主页提供的私密联系方式报告，并提供最小复现步骤。收到报告后，维护者会先确认影响范围，再安排修复与披露。

Do not open a public issue for vulnerabilities that may expose credentials, bypass permissions, or execute arbitrary code. Report them privately through the contact method on the repository owner's GitHub profile and include minimal reproduction steps. The maintainer will confirm impact before coordinating a fix and disclosure.

## 凭证安全 / Credential safety

- mimi 桌面正式版的 API Key 只保存在系统钥匙串中：macOS Keychain / Windows 凭据管理器 / Linux Secret Service。每个服务档案按「档案 ID + 服务商」独立存储，代码中没有明文、仓库或环境变量回退。
- 设置页与 IPC 只返回 `present`、`missing` 或 `unavailable`，不会读回或广播已保存的 Key。
- 不要在 Issue、Pull Request、日志或截图中包含 API Key；如果 Key 曾公开，请立即在对应服务商控制台重置或停用。

- Production mimi desktop builds store API keys only in the OS keychain: macOS Keychain / Windows Credential Manager / Linux Secret Service. Every service profile is isolated by profile ID and provider; there is no plaintext, source-controlled, or environment-variable fallback.
- Settings and IPC expose only `present`, `missing`, or `unavailable`, never the saved key itself.
- Never include API keys in issues, pull requests, logs, or screenshots. Reset or disable a key in its provider console immediately if it was exposed.

macOS 本机开发可明确启用默认关闭的 `local-dev-credentials`，仅固定 dev 应用在非 UI-only 模式读取经过验证的私有只读文件；不回退或迁移凭证。详见[本机开发凭证](docs/development/local-dev-credentials.md)。

Local macOS development may explicitly enable the default-off `local-dev-credentials` feature. Only the fixed dev app outside UI-only mode reads a validated private, read-only file, without credential fallback or migration. See [local development credentials](docs/development/local-dev-credentials.md).

## 数据与权限 / Data and permissions

- 字幕保留和系统音频录制默认关闭。开启后，已确认字幕或音频随会话写入有容量限制的本机私有文件；关闭选项会清空本次对应内容，已保存记录需主动删除。音频只发送到当前明确选中的识别服务；独立文字翻译只接收识别文字。
- macOS 仅请求「屏幕与系统音频录制」权限用于系统音频采集，不录制屏幕内容；Windows 使用 WASAPI 环回；Linux 只连接 PulseAudio / PipeWire-Pulse 输出设备的监听通道，不回退到默认输入设备。
- 诊断日志只包含计时、计数、语言码、状态码与错误标签，不包含识别或翻译文本。

- Subtitle saving and system-audio recording are off by default. When enabled, confirmed subtitles or audio are written incrementally to bounded private local files. Disabling an option clears its current-session content; saved sessions require explicit deletion. Audio goes only to the selected speech service; independent text translation receives recognized text only.
- macOS requests Screen & System Audio Recording access solely for system-audio capture (no screen content is recorded); Windows uses WASAPI loopback; Linux explicitly connects to the PulseAudio / PipeWire-Pulse output monitor and never falls back to the default input.
- Diagnostics contain only timings, counts, language codes, status codes, and error labels — never recognized or translated text.
