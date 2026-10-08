# 参与贡献 / Contributing

感谢你帮助改进 mimi。小而聚焦的 Pull Request 最容易审查和合并。

Thank you for improving mimi. Small, focused pull requests are the easiest to review and merge.

## 开始之前 / Before you start

1. 不要在 Issue、日志、测试或截图中提交任何服务商的真实 API Key。
2. Bug 请附上操作系统（macOS / Windows / Linux）与版本、复现步骤、预期行为和实际行为。
3. 较大的功能先创建 Issue，说明使用场景和体验目标。

1. Never commit a real provider API key in issues, logs, tests, or screenshots.
2. Bug reports should include the operating system (macOS / Windows / Linux) and version, reproduction steps, expected behavior, and actual behavior.
3. Open an issue before a large feature and explain the use case and UX goal.

## 从源码运行 / Run from source

需要 Rust 1.88+，以及 Node.js 20.19.x、22.13+ 或 24+。消回声库的源码构建还需要 C/C++ 编译器、CMake 和 libclang（仅构建时使用）。macOS 还需 Xcode Command Line Tools 和 `mimi Local Development` 签名身份，或显式设置 `MIMI_CODESIGN_IDENTITY`。

Requires Rust 1.88+ and Node.js 20.19.x, 22.13+, or 24+. Building the echo canceller also requires a C/C++ compiler, CMake, and libclang (build time only). macOS also needs the Xcode Command Line Tools and a `mimi Local Development` signing identity or an explicit `MIMI_CODESIGN_IDENTITY`.

```bash
git clone https://github.com/yuxino/mimi.git
cd mimi
npm ci
npm run tauri:dev        # Windows / Linux
./scripts/dev-app.sh     # macOS
```

macOS 可通过 `brew install cmake` 安装 CMake；Xcode Command Line Tools 提供编译器和 libclang。Windows 需要 Visual Studio 的 C++ 桌面开发工具、CMake 和 LLVM；若 bindgen 无法找到 `libclang.dll`，将 `LIBCLANG_PATH` 设为 LLVM 的 `bin` 目录。

On macOS, install CMake with `brew install cmake`; Xcode Command Line Tools supply the compiler and libclang. Windows needs Visual Studio Desktop development with C++, CMake, and LLVM. If bindgen cannot locate `libclang.dll`, set `LIBCLANG_PATH` to LLVM’s `bin` directory.

开发版和正式版都在设置中添加、编辑服务配置，并将凭据存入各自的私有本地文件。两边目录独立，开发版不读取 `.env`。启动和隔离规则见[开发凭据](../docs/development/local-dev-credentials.md)。

Development and production both use editable service configurations and private local credential files in separate app directories. Development does not read `.env`. See [development credentials](../docs/development/local-dev-credentials.md) for launch and isolation rules.

Windows 安装包需在 Windows 上构建 / Build Windows installers on Windows:

```bash
npm run tauri -- build --config src-tauri/tauri.ci.conf.json -- --locked
```

macOS 打包使用 / Package on macOS with `./scripts/package-app.sh`.

Linux 开发环境与安装包构建见 [Linux guide](../docs/development/linux.md)。Ubuntu 上先安装 `./scripts/linux-ci-deps.sh` 中列出的依赖，再运行 `npm run tauri:dev`。Linux CI 在独立会话中验证输出监听、Secret Service 和无凭据的 UI 启动。

For Linux dependencies, building, and isolated audio/keyring/UI checks, see the [Linux guide](../docs/development/linux.md). Packages are built on Ubuntu 22.04 for x86_64.

## 本地验证 / Local verification

界面改动前请阅读[界面一致性与操作反馈规范](../docs/development/ui-guidelines.md)。修复某个页面的重复问题时，需要排查其他页面的同类控件，并检查成功、失败、弹窗和异步结果迟到的情况。

Before UI changes, read [UI consistency and feedback](../docs/development/ui-guidelines.md). When fixing a repeated pattern, audit its sibling controls on other pages and check success, failure, modal and late-result paths.

```bash
./scripts/check.sh
./scripts/package-app.sh
```

界面改动还需要在 macOS 通过 `./scripts/dev-app.sh` 启动固定身份的应用（Windows 使用 `npm run tauri:dev`），检查设置窗、托盘面板和字幕浮窗的普通、空白、错误、暂停、收起、翻译中和长字幕状态。涉及延迟或流式管线的改动应使用所改服务商的真实会话（独立 Dev 配置中的本机私有凭据文件）验证并记录测量结果。

macOS 调试或打包前请阅读 [`docs/development/common-regressions.md`](../docs/development/common-regressions.md)。日常验证一律使用 `/Applications/mimi-dev.app`。替换正式版前必须比较完整签名要求；旧临时签名版本迁移到固定签名仍可能需要重新授权一次。公开发布请遵循[固定签名流程](../docs/development/macos-release-signing.md)。

For UI changes, launch the stable app identity with `./scripts/dev-app.sh` on macOS (`npm run tauri:dev` on Windows) and inspect the settings window, tray panel, and overlay in normal, empty, error, paused, collapsed, translating, and long-subtitle states. Latency- or streaming-sensitive changes should be verified against a real session for the changed provider, using the independent development app’s saved local credential file, and include measured results.

Before macOS testing or packaging, read [`docs/development/common-regressions.md`](../docs/development/common-regressions.md). Routine testing belongs in `/Applications/mimi-dev.app`. Compare complete designated requirements before replacing the formal app; migration from old ad-hoc releases may require one new grant. Follow the [fixed-signing release workflow](../docs/development/macos-release-signing.md) for public artifacts.

## 平台 / Platforms

- macOS、Windows 与 Linux 共用一套代码。平台差异集中在 `src-tauri/src/audio/`（macOS 用 ScreenCaptureKit，Windows 用 WASAPI loopback，Linux 用 PulseAudio / PipeWire-Pulse 输出监听）与凭证迁移（读取旧 macOS 钥匙串 / Windows 凭据管理器 / Linux Secret Service；日常使用私有本地文件）。
- Windows 打包请在 Windows 机器上执行；CI 会在 macOS、Windows 与 Linux 三个平台运行完整的 Rust 测试与 clippy。

macOS, Windows, and Linux share one codebase. Platform differences live in `src-tauri/src/audio/` (ScreenCaptureKit on macOS, WASAPI loopback on Windows, PulseAudio / PipeWire-Pulse output monitors on Linux) and legacy credential import (macOS Keychain / Windows Credential Manager / Linux Secret Service; ordinary credentials use private local files). Build the Windows package on a Windows machine; CI runs the full Rust tests and clippy on all three platforms.

## Pull Request

- 说明用户可见的变化和原因。
- 列出运行过的测试；界面改动附截图。
- 保持提交信息简洁，避免把无关重构混在一起。
- 确认 `git diff` 中没有凭证、录音、个人路径或构建产物。

- Explain the user-visible change and why it is needed.
- List tests run and include screenshots for UI changes.
- Keep commits focused and avoid unrelated refactors.
- Confirm the diff contains no credentials, recordings, personal paths, or build artifacts.
