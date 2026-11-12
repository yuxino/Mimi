# 共用服务说明

桌面和 Android 的服务能力、开通条件及阿里云模式说明，统一维护在
[`shared/provider-copy.json`](../../shared/provider-copy.json)。九种界面语言必须一起更新。

修改后运行：

```sh
python3 -B scripts/sync-provider-copy.py
python3 -B scripts/sync-provider-copy.py --check
```

桌面文案按语言生成到 `src/lib/generated/providerCopy/<locale>.ts`，
`src/lib/generated/providerCopy.ts` 只保留共用预设和服务入口元数据。
Android 生成各语言的 `provider_strings.xml`、`GeneratedTextTranslationPresets.kt`
和 `GeneratedServiceSetups.kt`。不要直接改生成文件，也不要把同一句复制回平台语言文件。
桌面设置的服务商帮助和阿里云模式选择、Android 服务介绍／配置帮助／模式选择
都读取这些结果；Android 的服务开通提示也放在这份目录里。

桌面语言模块只导入自己的服务文案；界面通过 `I18N.provider` 读取当前语言，
元数据消费者不要重新导入九语言全集。检查会拒绝缺少语言、错误的语言导入、
重复 Android 资源、失去桌面绑定以及未重新生成的文案。
阿里同传的翻译语言数取自共享协议目录，不在两端分别填写。

`compatiblePresets` 共用 OpenAI 兼容文字服务的名称、地址和示例模型。模型可为空，
由用户填写已开通或已下载的模型；预设只修改编辑草稿，不创建新协议、不保存或联网。
生成检查也覆盖预设字段、地址限制和两端输出。服务能力仍由共享 Rust 运行时决定。

每项预设还维护 `apiKeyUrl`、`guideUrl`、`setupKind` 和 `setupHelpKey`。帮助内容说明
从哪里创建 Key、模型如何填写及该服务必要的地域或产品限制；九种语言共用同一入口。
云端服务提供获取 Key 和配置指南，本机服务提供安装设置，免 Key 服务明确说明无需 Key。
设置页只在用户点击后打开目录中的固定官方 HTTPS 页面，不打开可编辑的接口地址，
不因选择预设而跳转、保存或检查连接。官方控制台的固定查询参数与片段可以保留；
它们不是翻译接口地址。Android 由生成的资源 ID 读取同一份帮助。

`serviceSetups` 维护内置语音服务和独立 DeepL/xAI 文字服务的凭据入口，
生成桌面 `SERVICE_SETUPS` 和 Android `GeneratedServiceSetups.kt`。补服务时
同时检查语音配置、独立文字配置与兼容预设，不能只补其中一处。火山语音、
百度语音翻译和腾讯语音的凭据分别来自豆包语音、机器翻译和 CAM；不要套用
同厂商 Ark、千帆或 TokenHub 的文字模型 Key。Azure 先进入资源控制台。
自定义接口未确定密钥签发方时不推测一个官方入口；本机和免 Key 服务沿用
各自的设置帮助。

维护预设时，对照共享协议契约和服务开通文档核查遗漏；预设数量不等于支持范围。
整链语音服务和同厂商的兼容文字 API 分开核对，并说明各自所需的凭据。

`scope` 决定生成到哪些平台；Android 专属开通说明不会进入桌面文案对象。
分段说明在 JSON 字符串中使用 `\n` 换行转义，不要写成 `\\n`，后者会显示为字面字符。
Apple 本地服务、自定义识别等仅桌面支持的功能，以及
录音权限、系统设置路径、导航、保存反馈等平台交互说明，仍由各平台维护。
不要为了共用一句话暗示另一端已有功能。

README 和服务开通教程负责入口、操作顺序与链接，不重复维护界面帮助全文。
服务能力改变时，先确认共享运行时及两端入口，再改这份目录；生成成功不代表
真实服务或原生界面已经验收。
