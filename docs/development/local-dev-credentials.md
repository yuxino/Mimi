# Local development credentials

## 日常使用

只需配置一次：把自己的测试 Key 写入
`~/Library/Application Support/app.yuxino.mimi.dev/.env` 的
`ALIBABA_API_KEY=` 或 `GEMINI_API_KEY=` 后面，不加引号，并将文件权限设为 `0600`。下面的
Setup 提供空模板的创建步骤；已有文件不需要重新填写。

之后每次运行 `./scripts/dev-app.sh`，或从 Finder / Dock 重新打开
`/Applications/mimi-dev.app`，应用都会自动读取该文件。设置中会有独立的
**Alibaba Cloud · dev** 默认开发配置，直接使用阿里云识别与翻译，无需再保存 Key。
文件中有 `GEMINI_API_KEY=` 时，还会出现独立的 **Google Gemini · dev** 预设。
两项密钥互不借用，空值表示该预设未配置。已有仅阿里云的文件不必改动。
修改 Key 后，正常退出并重新打开应用。

其他配置和正式版一样，可以添加、编辑和切换，也可以选择 DeepL、DeepLX、
ChatMock 或兼容接口作为独立文字翻译。它们使用开发版自己的系统钥匙串条目，
不会借用 `.env` 中的密钥，也不会改动正式版凭据。

把 `.env` 移走或删除，再重新打开，就切回系统钥匙串。文件存在但格式或权限错误时，
默认开发配置明确报错，不会偷偷改用钥匙串；其他配置仍可正常使用。
正式版和 `--ui-only` 模式都不会读取这个文件。
不要把真实 Key 放进仓库、截图或日志，也不要在 shell 中 `source` 这个文件。

The normal application stores provider credentials in the OS credential store.
For local macOS development, `./scripts/dev-app.sh` enables a separate, read-only
file-backed preset for each of Alibaba and Gemini so testing those presets does not require API-key Keychain access.
All other configurations use the development app's profile-scoped OS credential
store and remain editable. Their Keychain authorization follows the normal app
path; the preset's file key is never copied or used as their fallback.
This does not change the signing-private-key or system-audio permission prompts.

## Setup

1. Create the private file using the empty template. This preserves an existing
   file rather than overwriting its key:

   ```bash
   mimi_dev_config="$HOME/Library/Application Support/app.yuxino.mimi.dev"
   mkdir -p "$mimi_dev_config"
   chmod 700 "$mimi_dev_config"
   if [ ! -e "$mimi_dev_config/.env" ]; then
     (umask 077; cp docs/development/.env.example "$mimi_dev_config/.env")
   fi
   chmod 600 "$mimi_dev_config/.env"
   ```

2. Edit that private file in a local editor. Keep at most one assignment per provider, with an
   unquoted test key after `ALIBABA_API_KEY=` or `GEMINI_API_KEY=`. Do not put the real key
   in shell commands, Git, screenshots, diagnostics, or the tracked template.
   Do not `source` this file. Comments and blank lines are allowed; shell
   substitutions, quoted values, duplicate provider assignments, and other variables
   are rejected. It is not a general dotenv parser.

3. Run `./scripts/dev-app.sh` and use **Alibaba Cloud · dev**. The preset uses
   Alibaba recognition and default translation, including Original mode. It is
   selected initially when the existing selection is the default Alibaba profile;
   later explicit selections are retained across restarts. Add or edit ordinary
   configurations for other recognition or independent translation services.
   Those credentials always use the development OS store, never this file.
   Select **Google Gemini · dev** for the Gemini Live Translation preset; it uses
   only `GEMINI_API_KEY` and keeps its built-in translation route.

The file must be a regular, non-symlink file owned by the current user, with
exactly `0600` permissions and at most 16 KiB. A key is capped at 4096 bytes.
An empty template selects file mode with a missing key.

## Editing and returning to Keychain

Keys are read once at app startup. After changing the file, quit Mimi normally
and reopen `/Applications/mimi-dev.app`; reopening from Finder or the Dock also
reads it, because the path comes from the app config directory rather than the
launching shell. No polling or automatic Keychain retry occurs.

The built-in development presets are read-only: Settings cannot update, remove or
reveal their keys, change their recognition/translation services, or delete them.
Their language and proxy preferences remain editable. Ordinary profiles keep their
normal add/edit/delete, independent translation, credential reveal and switching
behavior, with at most 20 user configurations in addition to the presets.

To remove or replace a preset key, edit the private file and reopen the app.
Moving that file out of the fixed path or deleting it removes the preset on the
next startup. Removing only `GEMINI_API_KEY` removes the Gemini preset while
preserving Alibaba. An explicit active-profile selection survives restarts while
its preset is present. Ordinary profile metadata and Keychain entries are preserved.

If the file exists but cannot be read or fails validation, Mimi reports a local
development file error for the preset. It does **not** silently use Keychain.
Check its format, ownership and permissions, then reopen the app. Other profiles
continue using their own OS credentials even when the preset file is invalid.

## Scope and verification

This feature is disabled by default in Cargo. It additionally requires macOS
and the exact `app.yuxino.mimi.dev` bundle identifier before any secret-file
access. Release identifiers ignore it even if accidentally compiled with the
feature. UI-only mode never inspects the file, Keychain, provider networks, or
system audio. The file is not copied into `.app`, preferences, profile JSON, or
frontend state, and process environment variables are not credential inputs.

Run focused synthetic tests without using your key:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib local_dev_credentials
cargo test --manifest-path src-tauri/Cargo.toml --features local-dev-credentials --lib local_dev_credentials
```

Never commit a `.env` file. Only the empty `.env.example` template is tracked.
This is a local development exception, not a production credential-storage option.

For an explicitly authorized Gemini connection check without opening the GUI:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features local-dev-credentials --lib \
  local_dev_credentials_live_gemini_probe -- --ignored --nocapture
```

This invokes the same native speech-service check as Settings, loads only the
Gemini preset key, and reports credential/service status and elapsed time. It
does not capture audio or change the saved profile selection. It is ignored by
default because it contacts Google using the private local key.
