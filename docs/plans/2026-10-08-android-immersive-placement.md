# Android 沉浸入口、恢复与字幕位置

用户补充：手机和电脑的交互不同，Android 的沉浸入口藏在浮窗展开面板和设置里，
退出还要找独立按钮，首屏没有入口。此次仅修改 Android 原生呈现。具体偏移
方向、手机尺寸和原反馈的完整复现仍未知，不能把源码缺陷等同于真机根因。

## 源码证据与目标

交接快照的沉浸切换调用 hideOverlay → showOverlay，必然销毁已有展开视图、
滚动位置并回到紧凑态。展开使用临时 y=0，紧凑使用保存的 dp 偏移；再次展开
没有状态保护，可能把展开的 y=0 当作紧凑偏移。保存偏移最高 2000dp，未根据
当前字幕高度和可用显示区域约束；紧凑文字最大宽度只在创建时读取一次。
这些路径由源码确认；原用户手机的具体表现仍要设备复验。

- 首页在字幕预览旁直接提供沉浸／退出按钮，与设置、浮窗使用同一偏好和
  首次说明。入口不要求已经配置语音服务，也不会启动字幕会话。
- 切换在原有 WindowManager 窗口和文字视图上更新，不重建。保存的紧凑
  底部偏移与展开面板几何分离，重复通知不覆盖返回状态。展开进入沉浸，
  退出回到展开；首页紧凑进入，退出仍紧凑。保留历史阅读位置。
- 字幕高度增加时向上生长，底部锚点保持。空间不足只临时约束显示偏移；
  旋转回来恢复原 dp 偏好。仅用户完成字幕拖动才保存新偏移。
- 横屏的历史区增长也可能把同一当前句挤出阅读视口。读者仍跟随当前句时，
  历史布局变化后重新对齐当前句开头；当前句尚是草稿也适用。主动上翻阅读
  历史时保留滚动位置。原生 fixture 同时检查新增历史、长句末尾和历史回看。
- API 30+ 使用 overlay WindowContext 的 WindowMetrics 和 system-bar/cutout
  insets；让 WindowManager 忽略系统栏可见性进行一次 fitting。y 已是安全
  区域内的偏移，不再次加上导航栏高度。API 29 保留原生 visible display frame。
- 退出控件跟随字幕右侧，能放下时在字幕下方，否则在上方。至少 48dp
  触摸区域、宽度按语言／字体自适应。字幕仍透传触摸；控件采用小型原生
  companion window。移除原先独立的 42% 屏高定位和退出按钮拖动。

## 共享业务路径审查

Android 生产入口仍只有 SharedRuntimeEngine → JNI；旧 Kotlin engines 没有
生产构造调用。PC facades、客户端工厂、协议、PCM 队列、HQ MT 调度、Controller、
RecoverySchedule、MTBudgetContinuity、健康与 pending 期限保持共享 Rust。
atomicPreview 仍由实际工厂决定，两端 60ms 发布策略不变。连接检查保留同一
Rust probe，平台信任根／代理和采音仍是原生适配。没有复制字幕或翻译规则。

另一个可确认的生命周期差异：Android 服务把 provider 的 6000ms 超时用于
整个停止流程，未覆盖此前的 PCM 排空、之后的断连和快照通知。JNI 现在使用
与 PC 相同的 provider finish 上限，服务从共享 health 策略获得完整看门狗
预算：1000 + 6000 + 2000 + 2×60 = 9120ms。新增 JNI 合成事件测试检查停止
接纳当前可靠尾部、过滤旧 content revision 与 teardown 草稿，不连接服务。
此测试本轮因依赖缺失未执行；不把它记为通过。

## 云端与设备验证分界

`python3 scripts/check-android-overlay-placement.py` 编译实际生产 Java 定位类，
11 个场景通过；Android JUnit 复用同一组断言。PC 展示／chrome／最小高度／
resize 的 220 项聚焦回归、前端 lint／全套测试／类型与生产构建、6 项 updater
测试通过。Rust 格式检查通过，79 个 Android XML 解析及七语言资源绑定检查通过。

canonical 检查已实际运行：前置脚本通过，Cargo 阶段因 crates.io 下载受限且
没有 serde 缓存中止。没有 Android SDK、adb、emulator、AVD、KVM 或显示服务。
因此 Rust 编译／Clippy／测试、Android Gradle debug/release JVM/JNI/lint/APK、
instrumentation 和设备绘制都没有通过本轮验收。新增原生 fixture 待编译和运行，
不能把交接包中的旧 179 项测试和 APK 证据用于证明这次增量。

本地最短复验：先重跑 canonical 和 Android 两版构建／JNI 测试；在空闲无凭据
设备运行原生 overlay_interaction fixture，再检查首页入口、拖动后切换、展开
阅读恢复、连续切换、长字幕／大字体、横竖屏及手势／三键导航／全屏视频。记录
本轮 APK hash。具体网络／字幕延迟仍需同构建的无正文时间戳证据；未授权的新
采音、录制、真实服务调用不属于此次云端验证。
