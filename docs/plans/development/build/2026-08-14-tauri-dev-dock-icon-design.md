# mimi Tauri dev 模式 Dock 图标设计记录

> 2026-08-14。记录一次历时一整晚的图标问题排查的最终结论，避免重蹈覆辙。

## 结论（一句话）

**Tauri 的 `cfg(dev)` 不是构建类型，而是 `!custom-protocol` 特性。** 只要没有 `custom-protocol` 特性，即使是 `cargo build --release`，Tauri 也会把应用当 dev 处理，在运行时把 Dock 图标替换成未蒙版的方形图标。

## 问题现象

- release（`/Applications/mimi.app`）Dock 图标正确：圆角、正常大小。
- dev（dev-app.sh 包装的 .app）Dock 图标错误：**启动瞬间正确（圆角），应用完全启动后变成方形**。
- 修改 bundle 里的 `icon.icns` 无效；`setApplicationIconImage`、清图标缓存、重新注册 LaunchServices、换签名、换路径、补 Info.plist 键均无效。

## 根因机制（源码链）

1. **`cfg(dev)` 的定义** — `tauri/build.rs`：
   ```rust
   let custom_protocol = has_feature("custom-protocol");
   let dev = !custom_protocol;
   alias("dev", dev);
   println!("cargo:dev={dev}");
   ```
2. **dev 时的运行时图标覆盖** — `tauri/src/app.rs`（`RuntimeRunEvent::Ready` 分支）：
   ```rust
   #[cfg(all(dev, target_os = "macos"))]
   { /* NSApplication setApplicationIconImage(app_icon) */ }
   ```
   该 `app_icon` 来自 `context.app_icon`，由 `tauri-codegen` 在构建期嵌入：
   ```rust
   // tauri-codegen/src/context.rs
   let app_icon = if target == Target::MacOS && dev {
       find_icon(&config, ..., |i| i.ends_with(".icns"), "icons/icon.png")
   };
   ```
   即：**编译期嵌入 `src-tauri/icons/icon.icns` 的字节**（改 bundle 里的 icns 文件无效，因为用的是内嵌字节）。
3. **macOS 不对运行时设置的图标套圆角蒙版** — `setApplicationIconImage` 设置的图标按原样渲染：全出血的 icns 显示为**方形且更大**（这是 macOS 的已知行为，非 mimi 特有）。

## 为什么 `tauri build` 的产物是对的

`tauri build`（release）执行的构建命令是：
```
cargo build --bins --features tauri/custom-protocol --release
```
开了 `custom-protocol` → `dev=false` → 运行时覆盖不执行 → Dock 从 bundle 读 icns → macOS 正常蒙版 → 圆角。

## 修复

`scripts/dev-app.sh` 与 `tauri build` 保持一致：
```
cargo build --release --features tauri/custom-protocol --manifest-path src-tauri/Cargo.toml
```
dev 包装应用与 release 走完全相同的图标路径（bundle icns + 系统蒙版）。

## 附带要点

- **dev 标记**：dev 包装是 release 构建，`tauri::is_dev()` 恒为 `false`，所以 "(dev)" 标题/托盘提示改为按 bundle id `app.yuxino.mimi.dev` 判断（`windows::is_dev_build`）。
- **自定义 `set_dock_icon` 已删除**：它跑在 `setup()`，早于 Tauri 的 `Ready` 覆盖，永远被盖掉，无效。
- **调试期红鲱鱼**：图标缓存（`com.apple.iconservices*`、LaunchServices）、稳定签名、Info.plist 键、应用路径——都不是根因。
- 调试二进制（`npm run tauri dev` 裸跑）仍会触发该覆盖，Dock 图标无法蒙版；如需图标正确的调试体验，用 `scripts/dev-app.sh`。
