<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>把电脑或麦克风声音转成实时翻译字幕。</p>
  <p>Live translated subtitles from computer or microphone audio.</p>
  <p><a href="README_EN.md"><strong>English quick start →</strong></a></p>
  <p><a href="https://github.com/yuxino/mimi/releases/latest"><strong>下载 Mimi</strong></a> · <a href="https://mimi.yuxino.cn/">官网</a> · <a href="docs/usage.zh-CN.md">使用指南</a></p>
  <p>macOS 13+（Apple 芯片 / Intel） · Windows x64 · Linux x86_64 · <a href="android/README.zh-CN.md#start">Android 10+</a></p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="最新版本"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT 许可证"></a>
  </p>
  <p>
    <a href="README_EN.md">English</a> · <a href="docs/readme/zh-CN.md"><strong>简体中文</strong></a> · <a href="docs/readme/zh-TW.md">繁體中文</a> · <a href="docs/readme/ja.md">日本語</a> · <a href="docs/readme/th.md">ภาษาไทย</a> · <a href="docs/readme/ko.md">한국어</a> · <a href="docs/readme/fr.md">Français</a> · <a href="docs/readme/de.md">Deutsch</a>
  </p>
</div>

## 开始使用

Mimi 本身免费。使用云服务需要自己的密钥（API Key），服务商可能收费；采集的音频或识别文字会发给你选择的服务。

Apple 芯片 Mac（macOS 26+）也可使用 [Apple 本地服务](docs/provider-setup.zh-CN.md#apple-speech)，下载所需语言资源后无需 API Key。

1. **下载安装**：在[下载页](https://github.com/yuxino/mimi/releases/latest)选择对应系统的安装包。Mac 选 DMG，Windows 选 EXE，Linux 选 AppImage 或 DEB；手机看 [Android 安装说明](android/README.zh-CN.md#start)。
2. **配置服务**：打开「设置 → 模型与服务」，添加 OpenAI、Gemini、xAI 或其他支持的服务，填写凭据并保存。[配置指南](docs/provider-setup.zh-CN.md#choose)里有密钥获取和填写方法。
3. **选择声音和语言**：打开「声音与语言」，选择识别语言和翻译语言。默认使用电脑声音；想翻译自己说的话时，再选麦克风，也可以同时开启两路。
4. **开启字幕**：在「字幕」页打开「实时字幕」，按提示允许所需权限，再播放含有人声的视频、直播或其他内容。

![Mimi 双语字幕示意：原创插画与示例对白](docs/assets/readme-preview.png)

字幕显示示意，使用示例对白。[看字幕与沉浸模式演示（旧版界面）](docs/demos/README.md)。

## 常用功能

- 显示双语字幕，也可以只看译文或原文。
- 移动字幕、调整字号和颜色；沉浸模式只留文字，点击可穿透到下方应用。
- 桌面端可只听指定应用的声音（macOS / Windows 11），Linux 可选单独的播放流。
- 桌面端可手动开启字幕保存或录音，再导出 TXT / Markdown 字幕或 WAV 音频。默认不保存字幕和录音。

## 有问题？

安装、权限、显示和保存方法见[使用指南](docs/usage.zh-CN.md)。没有字幕时先看[识别错误](docs/usage.zh-CN.md#errors)，仍有问题可[提交反馈](https://github.com/yuxino/mimi/issues)。

<details>
<summary>贡献者与社区致谢</summary>

## 贡献者

想帮 Mimi 改进？从[贡献指南（中英）](.github/CONTRIBUTING.md)开始。

感谢每一位写代码、提问题、试用和分享的朋友 (๑•̀ㅂ•́)و✧

特别感谢 [@yebuwudong](https://github.com/yebuwudong) 贡献 [Android 版](https://github.com/yuxino/mimi/pull/37)，以及 [@LLLin000](https://github.com/LLLin000) 贡献[字幕动效](https://github.com/yuxino/mimi/pull/67)和 [Windows 音源改进](https://github.com/yuxino/mimi/pull/89)。

也感谢 [@Chtholly000](https://github.com/Chtholly000) 改进 [Gemini 连续字幕和连接轮换](https://github.com/yuxino/Mimi/pull/176)。

感谢 [@EmmetZ](https://github.com/EmmetZ) 修复 [Wayland 浮窗与控制组件不同步的问题](https://github.com/yuxino/Mimi/pull/242)，并新增 [Linux 单独播放流采集](https://github.com/yuxino/Mimi/pull/275)和[播放流应用图标](https://github.com/yuxino/Mimi/pull/304)。

感谢 [@wangmian0](https://github.com/wangmian0) 为 [DeepSeek 实时字幕翻译关闭思考模式](https://github.com/yuxino/Mimi/pull/270)。

感谢 [@hubumulity](https://github.com/hubumulity) 贡献[字幕显示条数设置](https://github.com/yuxino/Mimi/pull/303)。

感谢 [@ryannyui](https://github.com/ryannyui) 贡献 [Linux AppImage 图形兼容性修复和工具链升级](https://github.com/yuxino/Mimi/pull/312)。

<p>
  <a href="https://github.com/yuxino"><img src="docs/assets/contributors/yuxino.svg" width="40" height="40" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="docs/assets/contributors/LLLin000.svg" width="40" height="40" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="docs/assets/contributors/yebuwudong.svg" width="40" height="40" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="docs/assets/contributors/Chtholly000.svg" width="40" height="40" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="docs/assets/contributors/inhome.svg" width="40" height="40" alt="@inhome"></a>
  <a href="https://github.com/EmmetZ"><img src="docs/assets/contributors/EmmetZ.svg" width="40" height="40" alt="@EmmetZ"></a>
  <a href="https://github.com/wangmian0"><img src="docs/assets/contributors/wangmian0.svg" width="40" height="40" alt="@wangmian0"></a>
  <a href="https://github.com/hubumulity"><img src="docs/assets/contributors/hubumulity.svg" width="40" height="40" alt="@hubumulity"></a>
  <a href="https://github.com/ryannyui"><img src="docs/assets/contributors/ryannyui.svg" width="40" height="40" alt="@ryannyui"></a>
</p>

[查看所有贡献者](https://github.com/yuxino/mimi/graphs/contributors)

## 社区致谢

也感谢 [V2EX](https://www.v2ex.com/)、[LINUX DO](https://linux.do/)、[小众软件](https://meta.appinn.net/)、[NodeLoc](https://www.nodeloc.com/)、[Solo](https://solo.xin/)、[新趣集](https://xinquji.com/posts/859305)和[电鸭](https://eleduck.com/)社区朋友的试用、反馈与分享。

</details>

[MIT](LICENSE) © 2026 yuxino
