# Development credentials

开发版和正式版采用相同的可编辑配置与私有本地凭据文件设计，不读取 `.env`，
也不再自动创建 Alibaba / Gemini 开发预设。在设置的服务页面添加配置、保存凭据即可。
旧预设元数据如果仍在目录中，作为普通配置显示；原 `.env` 密钥不会自动导入。

## 启动与隔离

使用 `./scripts/dev-app.sh` 构建并启动固定的 `/Applications/mimi-dev.app`。
保留既有签名和 bundle identifier，避免破坏系统录音授权连续性。

| 应用 | macOS 配置目录 |
| --- | --- |
| 正式版 `app.yuxino.mimi` | `~/Library/Application Support/app.yuxino.mimi/` |
| 开发版 `app.yuxino.mimi.dev` | `~/Library/Application Support/app.yuxino.mimi.dev/` |

每个目录分别保存 `preferences.json`、`service-profiles.json` 和私有的
`credentials/credentials.json`。凭据目录权限为 `0700`，文件为 `0600`。
启动、切换和编辑不会自动复制另一应用的数据；需要初始化副本时，必须明确指定，
保留原数据并转换凭据文件内的应用服务命名空间。复制完成后两边独立保存。
不要将真实密钥写入环境变量、命令行、Git、诊断、截图或导出。
旧 `.env` 可以离线备份或自行删除，运行时不会访问它。

Development uses the same editable local credential store as production, scoped
by its own app identifier and directory. No `.env` loader or environment-backed
preset remains. UI-only mode continues to use synthetic, in-memory credentials.
Legacy OS-store import is a one-time compatibility path; a completed import never
falls back to native credentials. See [local credential storage](../plans/2026-10-04-local-credential-storage.md).

## 手动服务探针

付费探针仍默认忽略，仅在 macOS 测试构建启用 `development-debugger` 时编译。
它们使用开发版**当前选择**的普通服务配置及已保存凭据；先在设置中选好对应
Alibaba 或 Gemini 配置，并停止原生应用中的服务会话。

探针要求本地凭据文件和完成标记已经存在，避免以探针触发旧系统凭据导入。
环境变量仅提供非秘密的 manifest、输出路径和测试模式。运行付费探针仍需当次明确授权；
普通自动检查不发送真实服务请求。

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features development-debugger --lib --no-run
```

ASR 比较和批次执行方法见 [ASR benchmark](../plans/2026-10-02-audio3-repeatable-benchmark.md)
及 [audio replay](audio-quality-replay.md)。签名私钥访问和系统录音权限独立于服务凭据存储。
