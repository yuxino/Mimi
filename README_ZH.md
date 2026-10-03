<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>mimi</h1>
  <p>系统音频实时字幕与翻译，支持 Apple 芯片和 Intel Mac（macOS 13+），以及 Windows / Linux x86_64。</p>
  <p>
    <a href="https://mimi.yuxino.cn">官网</a>
    · <a href="https://github.com/yuxino/mimi/releases/latest"><strong>下载最新版</strong></a>
    · <a href="README.md">English</a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="最新版本"></a>
    <a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="总下载量"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="main 分支 CI 状态"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT 许可证"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>
    <a href="android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
</div>

Mimi 为电脑上播放的电影、直播、网课和游戏显示实时悬浮字幕。你选择的云服务会识别所选输入的音频，或将其翻译成当前服务支持的语言。`mimi` 在日语中意为“耳朵”。

<!-- project-demo-v1 -->
## 演示

https://github.com/user-attachments/assets/342c049c-1bed-44da-b4d2-ba55dad1a49c

<p align="center">在 macOS 应用中演示服务配置、字幕控制和沉浸模式。4K / 60 帧，含中文旁白、字幕与电影原声。</p>
<p align="center"><a href="https://mimi.yuxino.cn/en/?lang=en#demo">Watch in English</a> · <a href="https://mimi.yuxino.cn/?lang=zh#demo">观看中文版</a> · <a href="docs/demos/full-tour-4k.md">演示说明与来源</a></p>
<!-- /project-demo-v1 -->

## 功能

- **实时字幕与翻译** — 系统声音默认开启。桌面麦克风输入及其操作入口暂时不可用；启动时会把旧的麦克风或双路输入设置改为系统音频，并关闭录音，需要录制时请重新明确开启。
- **服务配置** — 保存并切换多套服务配置，无需反复填写凭证。
- **字幕浮窗** — 支持移动、缩放、收起、暂停、点击穿透和沉浸模式。新安装默认尺寸为 659×328、字号为 16；已有窗口尺寸和字号设置会保留。
- **版本更新** — macOS、Windows 安装版和 Linux AppImage 可在设置中检查并安装更新；Windows ZIP 和 Linux .deb 提供 Releases 手动更新入口。
- **保存与导出** — 在「设置 → 保存与导出」选择保存已确认的字幕或将系统音频录制为 WAV。可查看已保存的记录，并导出 TXT / WAV 或删除记录；两个开关默认关闭。
- **设置外观** — 支持浅色、深色和跟随系统。
- **隐私** — 无需 mimi 账号，不录制屏幕画面；音频只发送给当前服务商。字幕保留和录音默认关闭，开启后会随会话写入有容量限制的本机私有文件。关闭开关会清空本次对应内容；已保存的记录会保留，需自行删除。

字幕最多保留 10,000 条已确认记录／2 MiB 文字，音频最多 64 MiB；达到上限会停止保留并提示。字幕时间戳表示确认时间，并非媒体播放位置。WAV 不包含暂停和重连间隙，因此不能与字幕时间戳直接对齐。

## 开始使用

1. 从 [Latest Release](https://github.com/yuxino/mimi/releases/latest) 下载 macOS Apple Silicon 或 Intel DMG，Windows x64 EXE、MSI、绿色版 ZIP，或 Linux x86_64 .deb / AppImage；也可以从源码构建。
2. 打开「翻译服务」，选择服务商并保存凭证。
3. 播放内容，从菜单栏/系统托盘的 mimi 图标点击 **开始**。macOS 会请求「屏幕与系统音频录制」权限，以采集系统音频。

需要自备服务商 API 凭证，调用可能产生费用。凭证保存在系统钥匙串中。

macOS、Windows 安装版和 Linux AppImage 更新时，打开 **设置 → 通用 → 版本更新**。Mimi 会显示下载进度，下载完成后可直接安装。
Windows 安装完成后会重新打开 Mimi；macOS 和 Linux AppImage 可点击 **重新启动并完成更新**。
早于 v1.3.8 的旧版本需要先手动安装一次，之后即可在应用内更新。

### 平台支持

- **macOS 13+（Apple 芯片和 Intel）**：Apple 芯片选择 `_aarch64.dmg`，Intel 选择 `_x64.dmg`。从 v1.4.4 起提供 Intel 包，已通过构建和签名检查，Intel 实机采集和权限行为仍待验证。提供未经 Apple 公证的 DMG；若首次打开被拦截，请在「系统设置 → 隐私与安全性」中选择「仍要打开」。从旧版本升级时，请留意下方的权限说明。
- **Windows x64**：提供未签名的预览版 EXE / MSI；从 v1.4.3 起还提供绿色版 ZIP。SmartScreen 可能显示提示。下载 `mimi_<version>_x64-portable.zip` 解压后，直接运行 `mimi.exe`。电脑需已安装 WebView2（Windows 11 通常自带）。ZIP 不会把设置、服务凭证或已导出文件搬到自身目录；它们仍保存在原来的用户目录、系统凭据管理器或用户选择的位置。更新绿色版时，先退出 Mimi，再从 Releases 下载新版 ZIP 替换；绿色版不会运行应用内安装器更新。

- **Linux x86_64 预览版（以 Ubuntu 22.04+ 为基线）**：提供 `.deb` 和 AppImage。需要 PulseAudio 或启用了 `pipewire-pulse` 的 PipeWire、可用的默认输出设备，以及已解锁的 Secret Service 密钥环（例如 GNOME Keyring）。系统音频只使用输出设备的监听源。切换输出设备后请重新开始会话。建议使用 X11；Wayland 的窗口定位、置顶和点击穿透可能受桌面环境限制；快捷键请使用 Mimi 设置里显示的命令，在系统键盘设置中配置。没有托盘图标时可从设置窗口操作；最小化会继续运行，关闭设置窗口则退出 Mimi。暂不提供 Linux ARM64 安装包。详见 [Linux 安装与验证](docs/development/linux.md)。

### macOS 更新后重复授权

发布流程现已要求 macOS 各版本沿用同一张固定自签名证书。v1.4.1 及更早的发布版使用随构建变化的临时签名；首次升级到固定身份的版本时，可能需要重新授予一次录音权限。源码中的修复不会改变已经安装的旧版本。固定签名消除了每次构建更换身份的原因，但不代表 macOS 永远不会再询问授权。删除本地签名证书不会改变已安装应用的身份，也不能解决这个问题。正式版请放在 `/Applications/mimi.app`，开发测试使用独立的 `mimi-dev.app`。

**录音权限已开启，但 Mimi 仍提示无权限：**先退出并重新打开 Mimi，按正常提示授权。如果仍无法采集：

1. 退出 Mimi。打开「系统设置 → 隐私与安全性 → 屏幕与系统音频录制」（名称随 macOS 版本略有不同），只选中并移除旧的 **mimi** 条目。
2. 点击 **+**，重新添加 `/Applications/mimi.app` 并开启权限；系统要求认证时由你本人完成。不要移除 `mimi-dev` 或其他应用。
3. 重新打开同一个应用，开始会话并播放带人声的音频，确认实际出现字幕。仅看到权限开关开启，不代表已经恢复。

这里移除的是旧录音授权，不是证书或 API Key。尝试一次仍未恢复，请停止反复重置，携带 macOS 版本、Mimi 版本、安装来源和错误提示[反馈问题](https://github.com/yuxino/mimi/issues)，不要附带 API Key 或字幕内容。

**钥匙串要求密码或访问已保存的 API Key：**先确认请求来自你准备使用的 Mimi。如果弹窗提供「始终允许」，它可以为当前构建保留访问许可，但无法保证自签名应用更新后不再询问。不要删除已保存的凭证、证书或登录钥匙串，不要允许所有应用访问密钥，也不要重置整个系统的权限。`codesign` 请求访问开发签名私钥是另一种构建时弹窗。

作者的钱包还瘪瘪的，正在攒钱开通 Apple 开发者会员 (๑•̀ㅂ•́)و✧ 非常感谢你的理解！固定自签名不需要付费会员，Apple 公证是后续另一件事。上面的手动处理仅用于旧录音授权失效，不应该成为每次更新的固定步骤。

## 开发

构建与贡献请参阅 [贡献指南](CONTRIBUTING.md)，安全问题请参阅 [安全政策](SECURITY.md)。

## 字幕显示

桌面端可在设置中点选五种预设色块（默认白色），或点击预设后面的自定义色块打开系统调色板。预览与悬浮字幕立即更新，沉浸模式同样生效。双语模式下，原文与译文沿用该声音来源的所选颜色。

在设置、浮窗控制面板或托盘中选择 **仅译文**、**原文＋译文** 或 **仅原文**。
macOS 使用 **⌘⇧B**，Windows/Linux X11 使用 **Ctrl+Shift+B** 快速切换，不会中断翻译。Wayland 请在系统设置中为 `mimi --cycle-subtitle-display` 绑定快捷键。
双语模式成对显示已确认的原文与译文，等待翻译时先预览识别到的原文。

## 贡献者

感谢每一位参与 Mimi 的朋友。想一起改进？欢迎阅读 [贡献指南](CONTRIBUTING.md)。

特别感谢 [@yebuwudong](https://github.com/yebuwudong) 在 [PR #37](https://github.com/yuxino/mimi/pull/37) 中贡献原生 Android 版本。[下载 Android](https://github.com/yuxino/mimi/releases/latest)，配置方式和已验证范围见 [Android 说明](android/README.md)。

<p>
  <a href="https://github.com/yuxino"><img src="docs/assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/inhome"><img src="docs/assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
  <a href="https://github.com/LLLin000"><img src="docs/assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="docs/assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
</p>

[查看 GitHub 贡献记录](https://github.com/yuxino/mimi/graphs/contributors)。头像墙按默认分支的公开贡献归属核对，尚未合入的工作另列。

正在审阅：[@LLLin000 的字幕动效 #67](https://github.com/yuxino/mimi/pull/67) 与 [Windows 音源改进 #89](https://github.com/yuxino/mimi/pull/89)，已纳入 [集成验收 #88](https://github.com/yuxino/mimi/pull/88)。

## 社区友链

[LINUX DO](https://linux.do/)

[MIT](LICENSE) © 2026 yuxino
