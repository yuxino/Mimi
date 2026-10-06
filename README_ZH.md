<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>mimi</h1>
  <p>系统声音或麦克风实时字幕与翻译，支持 Apple 芯片和 Intel Mac（macOS 13+），以及 Windows / Linux x86_64。</p>
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

<p align="center">Mimi 把电脑或麦克风中的人声翻译成实时字幕。看电影、直播或网课时，字幕会悬浮显示在屏幕上。</p>

![Mimi 双语字幕窗口，搭配原创插画与示例对白](docs/assets/readme-preview.png)

## 功能

- 选择系统声音、麦克风，或同时开启两路。默认使用系统声音，麦克风需要主动选择。
- 选择要翻译的应用声音（macOS / Windows 11）。
- 显示原文、译文，或双语字幕。
- 调整字幕的位置、大小和颜色，也可让鼠标点击穿过字幕窗口。
- 按需保存字幕或音频到本机，导出 TXT / WAV；默认不保存、不录音。

## 开始使用

初次使用，**推荐先用阿里云或火山引擎**。我们对阿里云做了更多实际测试，火山引擎用下来效果也不错。其他服务还在继续完善，使用效果和稳定性可能有差异。

也可以试试 Google Gemini，网络连接稳定时体验不错。目前提供免费额度，额度和计费以 [Google AI Studio](https://aistudio.google.com/) 显示为准；也可以在那里获取 API Key。

各家服务的开通入口、凭证获取步骤，以及 Mimi 当前使用的模型，见**[服务开通指南](docs/provider-setup.zh-CN.md)**。

1. 打开「设置 → 语音与翻译」，添加配置，按提示填入服务商凭证并保存。
2. 选择识别和翻译语言。
3. 播放内容，在「字幕」页开启「实时字幕」。macOS 提示时，允许「屏幕与系统音频录制」。

云端识别需要自备服务商凭证，并将音频发送至该服务，调用可能产生费用。符合条件的 Mac 可用 Apple Speech 在本地识别；独立文字翻译会将识别文字发送至你选择的翻译服务。

[使用与常见问题](docs/usage.zh-CN.md) · [Android](android/README.md) · [反馈问题](https://github.com/yuxino/mimi/issues) · [贡献指南](CONTRIBUTING.md)

### Apple 本地识别

**Apple Speech** 只在系统支持时出现：Apple 芯片、macOS 26 或更新版本，且系统识别引擎可用。识别不需要 API Key。先停止字幕，在 Mimi 的配置中点击「添加语言包」，选择语言并点击「下载语言包」；准备好后点击「设为识别语言」，再启动字幕。不提供自动识别语言。详见 [Apple Speech 配置步骤](docs/provider-setup.zh-CN.md#apple-speech)。

可以开启「跳过翻译」只做识别，也可单独配置 Index-Translate 等文字翻译服务。Apple 识别本身不提供翻译模型；选择远程翻译服务时，识别文字仍会发送至该服务。

### 试试 Index-Translate

B 站的 [Index-Translate](https://github.com/bilibili/Index-Translate#inference) 目前提供免费的公开翻译 API（截至 2026 年 10 月 5 日）。可以接到 Mimi 里试试，对比一下和常用服务的字幕翻译效果。

在「**设置 → 语音与翻译**」中打开「**Alibaba Cloud**」配置，将「**文字翻译**」的服务切换为「**OpenAI 兼容接口**」，按[官方示例](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py#L40-L41)填写：

| 字段 | 填写内容 |
| --- | --- |
| 服务地址 | `https://index-translate.bilibili.com/v1` |
| 模型名称 | `Index-Translate-35B-A3B` |
| API Key | 留空，公开接口目前不需要认证。 |

保存后，点击「文字翻译」旁的连接检查。如果这个地址之前保存过 Key，选择「移除翻译密钥」后再保存。

Index-Translate 只负责文字翻译，仍需配置阿里云语音识别凭证，识别服务可能产生费用。免费接口的后续可用性以上游为准。

## 常见问题

**macOS 已开启录音权限，仍反复要求授权？** 先退出 Mimi，在「系统设置 → 隐私与安全性 → 录屏与系统录音」中，仅删除并重新添加对应应用：正式版为 `/Applications/mimi.app`，开发版为 `/Applications/mimi-dev.app`，开启权限后重新打开同一个应用。详见[权限恢复步骤](docs/usage.zh-CN.md#macos-更新后重复授权)。

## 贡献者

感谢每一位写代码、提问题、试用和分享的朋友 (๑•̀ㅂ•́)و✧

特别感谢 [@yebuwudong](https://github.com/yebuwudong) 贡献 [Android 版](https://github.com/yuxino/mimi/pull/37)，以及 [@LLLin000](https://github.com/LLLin000) 贡献[字幕动效](https://github.com/yuxino/mimi/pull/67)和 [Windows 音源改进](https://github.com/yuxino/mimi/pull/89)。

也感谢 [@Chtholly000](https://github.com/Chtholly000) 改进 [Gemini 连续字幕和连接轮换](https://github.com/yuxino/Mimi/pull/176)。

<p>
  <a href="https://github.com/yuxino"><img src="docs/assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="docs/assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="docs/assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="docs/assets/contributors/Chtholly000.svg" width="64" height="64" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="docs/assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
</p>

[查看所有贡献者](https://github.com/yuxino/mimi/graphs/contributors)

## 社区致谢

也感谢 [V2EX](https://www.v2ex.com/)、[LINUX DO](https://linux.do/)、[小众软件](https://meta.appinn.net/)、[NodeLoc](https://www.nodeloc.com/)、[Solo](https://solo.xin/)、[新趣集](https://xinquji.com/posts/859305)和[电鸭](https://eleduck.com/)社区朋友的试用、反馈与分享。

[MIT](LICENSE) © 2026 yuxino
