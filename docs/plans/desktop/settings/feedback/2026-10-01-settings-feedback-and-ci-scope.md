# 设置反馈与日常 CI

用户提供五张实际 Mac 私人配置截图。只用于本地像素审查，不公开。可见问题：Mac 展开 Debian/Ubuntu/AppImage、HTTP 和 UI-test 实现说明；连接检查结果冗长且与凭据错误重复；「名称与删除」合并普通编辑和危险动作；桌面候选「初次使用？」固定在删除附近。

设置改为单独的连接检查结果和折叠详情，不常驻开发/测试说明。按当前平台显示恢复操作，Mac 不显示 Linux 安装建议；服务器可达只标授权尚未验证，绝不等同认证成功。检查仍由用户主动点击，保留原生安全存储错误及重试行动，不更改凭据读取、权限或收费路径。配置名称与危险删除分开，保留确认、最后一个配置不可删和会话锁定。桌面引导入口在独立候选中移到设置页标题区域，与连续教程连接；不借此自动合入 #85。

## 耗时依据

实际 GitHub run 36817701101（da19375）：frontend0.60min，WindowsARM64 14.45min，平台bundle6.43–8.33min，三平台Rust5.27–8.43min。
run36818030304（0dfb118）：frontend0.55min，WindowsARM64 9.47min，bundle4.93–6.98min，Rust1.45–3.63min。只是两次测量，缓存/排队不同，不承诺固定提速比例。

## 触发矩阵

| 触发 | 编译/测试 | 原生包 |
|---|---|---|
| PR / main日常前端、文案、文档 | 前端lint/tests/typecheck/build、既有脚本检查 | 默认无 |
| Windows/macOS/Linux专有采音代码 | 对应平台Rustfmt/clippy/tests；Windows含ARM64调试编译/tests | 默认无 |
| 共享Rust、依赖、权限、构建脚本或workflow | 所有受影响平台（保守选择全部）Rust/MSRV，前端 | 默认无；ARM64不默认编译release程序 |
| 手动validation=full | 三平台+ARM64/MSRV全部检查 | 仍可不打包 |
| 手动packages=linux/windows/macos/all | 该平台包与既有真实包smoke；Windows含ARM64编译/启动 | 只所选平台，all为显式完整验收 |
| 正式v* tag | 现有全平台检查、固定签名、版本/来源和发布校验 | 现有全平台正式发布路径 |

范围相对 PR base 的完整差异判断，不能用未经证明的上一 SHA 绿灯跳过仍在 PR 中的原生改动。diff读取失败保守扩大检查，新增gate要求所有计划执行的job成功；不改变分支保护，不扩大token权限。手动模式的默认值是quick+none。Android日常保留Debug/Release单测、双lint、instrumentation APK编译；验收APKs与签名拒绝fixture移至显式手动packages或正式tag，正式android-release签名/provenance逻辑保持不动。

正在验收的旧准确安装包和工件不受workflow变化影响，不改包身份、不重装用户正在使用的应用。检查工具、范围规则测试、前端回归和远程准确SHA必须分别报告；新workflow未运行不能冒称远程优化已通过。
