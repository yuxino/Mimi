# Linux 播放流图标的桌面入口消歧

## 依据与问题

PR #304 的 executable 分支取第一个桌面入口。多个浏览器配置、解释器
应用或 native／Flatpak 安装可以使用同一个执行程序，因此排序会决定
显示谁的图标，甚至跳过准确名称。名称本身也可能重复。

- [GIO executable](https://docs.gtk.org/gio/method.AppInfo.get_executable.html)
  只描述执行程序名，不承诺唯一身份。
- [Desktop Entry 规范](https://specifications.freedesktop.org/desktop-entry/latest-single/)
  区分 desktop file ID、本地化 Name 和可带参数的 Exec。
- [Flatpak 约定](https://docs.flatpak.org/en/latest/conventions.html#desktop-files)
  将原命令放入 `--command`，允许 `ID.desktop`、`ID.foo.desktop` 和
  `ID-foo.desktop`。原实现误拒绝后两种入口。

## 决定

这是图标关联的工程规则，规范没有定义音频流到桌面入口的关联算法。
仅修改 Linux 展示适配器，不改捕获令牌、选择状态或保存的数据。

1. 明确 application ID 命中仍优先；入口缺少图标时使用默认图标。
2. 合并普通 executable 与 Flatpak 声明命令的候选。在候选中按准确
   desktop ID／名称消歧；仍有多个候选则默认回退，不按列表顺序猜测。
3. 没有 binary 候选时，依次尝试名称对应的 desktop ID、准确 Name、
   executable 别名和唯一版本后缀。已有准确名称歧义时，不降级挑选版本。
4. 不按有无图标过滤候选，不执行启动器，不根据沙箱 PID 查找宿主进程。
5. Flatpak 后缀只接受官方的点／连字符边界；保留 `X-Flatpak`、
   `flatpak run` 与 app 参数检查。

## 验证

真实 GIO 夹具覆盖重复执行程序、交换顺序、名称缺失／冲突、同名不同
程序、明确 ID 优先、native／Flatpak 交叉候选、合法与非法 Flatpak 后缀。
原有唯一候选、D-Bus-only 和版本候选测试继续运行。

Linux CI 额外用 Xvfb 精确执行现有原生 GTK PNG／SVG 渲染及预算回归，
先检查测试存在，避免过滤到零项假绿。本机 macOS 检查不替代 Linux
GIO／GTK 证据；该夹具也不代表真实桌面主题、音频或提供商验收。
