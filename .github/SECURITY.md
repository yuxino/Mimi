# 安全策略 / Security Policy

## 报告漏洞 / Reporting a vulnerability

请不要公开提交可能泄露凭证、绕过权限或执行任意代码的安全问题。请通过 GitHub 仓库所有者主页提供的私密联系方式报告，并提供最小复现步骤。收到报告后，维护者会先确认影响范围，再安排修复与披露。

Do not open a public issue for vulnerabilities that may expose credentials, bypass permissions, or execute arbitrary code. Report them privately through the contact method on the repository owner's GitHub profile and include minimal reproduction steps. The maintainer will confirm impact before coordinating a fix and disclosure.

## 凭证安全 / Credential safety

- mimi 桌面端的 API Key 保存在独立于偏好设置的本机私有明文文件中，由文件权限保护；同一用户下运行的程序仍可能读取。每个服务档案按「档案 ID + 服务商」独立存储，正式版不从仓库或环境变量读取凭据。
- 旧 macOS Keychain / Windows 凭据管理器 / Linux Secret Service 条目只用于一次自动迁移；本地写入并读回校验成功后，才清理对应的 Mimi 旧条目。完成后只使用本地文件，文件缺失或损坏也不回退到旧系统存储。
- 设置快照与广播只含 `present`、`missing` 或 `unavailable` 等凭据状态。用户主动查看已保存凭据时，仅设置窗口可通过专用 IPC 读取；显示值在失焦或隐藏后清除。
- 不要在 Issue、Pull Request、日志或截图中包含 API Key；如果 Key 曾公开，请立即在对应服务商控制台重置或停用。

- Mimi desktop stores API keys in a private plaintext file separate from preferences, protected by local file permissions; programs running as the same user may still read it. Every service profile is isolated by profile ID and provider. Production builds do not read credentials from the repository or environment variables.
- Existing macOS Keychain / Windows Credential Manager / Linux Secret Service entries are used only for a one-time automatic import. Mimi removes its corresponding old entries only after a durable local write and read-back verification. After import, only the local file is used; missing or damaged files do not fall back to the old system store.
- Settings snapshots and broadcasts contain only credential states such as `present`, `missing`, or `unavailable`. An explicit saved-credential reveal uses a dedicated IPC command restricted to the settings window; the displayed value is cleared on blur or hide.
- Never include API keys in issues, pull requests, logs, or screenshots. Reset or disable a key in its provider console immediately if it was exposed.

开发版使用相同的私有本地凭据文件设计，目录与正式版分开；不读取 `.env`，也不自动共享正式版配置或凭据。详见[开发凭据](../docs/development/local-dev-credentials.md)。

Development uses the same private local credential-file design in a separate app directory. It does not read `.env` or automatically share production configuration or credentials. See [development credentials](../docs/development/local-dev-credentials.md).

## 数据与权限 / Data and permissions

- 字幕保留和所选来源的音频录制默认关闭。开启后，已确认字幕或音频随会话写入有容量限制的本机私有文件；关闭选项会清空本次对应内容，已保存记录需主动删除。音频只发送到当前明确选中的识别服务；独立文字翻译只接收识别文字。
- 默认选择系统声音，麦克风需要主动选择。系统音频采集在 macOS 使用「屏幕与系统音频录制」权限，不录制屏幕内容；Windows 使用 WASAPI 环回；Linux 连接 PulseAudio / PipeWire-Pulse 输出设备的监听通道。系统来源不会回退到麦克风。麦克风使用系统默认输入设备，实际采集时才按需申请权限。
- 诊断日志只包含计时、计数、语言码、状态码与错误标签，不包含识别或翻译文本。

- Subtitle saving and recording of selected audio inputs are off by default. When enabled, confirmed subtitles or audio are written incrementally to bounded private local files. Disabling an option clears its current-session content; saved sessions require explicit deletion. Audio goes only to the selected speech service; independent text translation receives recognized text only.
- System audio is selected by default; microphone capture requires explicit selection. For system-audio capture, macOS uses Screen & System Audio Recording access without recording screen content, Windows uses WASAPI loopback, and Linux connects to the PulseAudio / PipeWire-Pulse output monitor. The system source never falls back to the microphone. Microphone capture uses the system's default input device and requests permission when capture starts, if needed.
- Diagnostics contain only timings, counts, language codes, status codes, and error labels — never recognized or translated text.
