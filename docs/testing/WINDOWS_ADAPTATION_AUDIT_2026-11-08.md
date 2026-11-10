# Windows 适配复查（2026-11-08）

基线 `0cdada8`，本轮协调见 [分工表](../WINDOWS_GUI_FIXES_COORDINATION_2026-11-08.md)。状态以代码、实际运行和最终集成报告为准，不能把历史开发计划中的“未实现”直接当作当前缺口。

## 本轮有明确证据的问题

| 问题及代码证据 | 处理人 / 独立分支 | 验收要求 |
| --- | --- | --- |
| `os-hal/src/platform/screen.rs::capture_region_below` 原先仅 macOS 排除遮罩，Windows 普通抓屏会摄入透明 WebView 的黑底或遮罩；用户确认 GIF 预览和产物黑屏 | Qoder / `codex/windows-gif-capture` | 真实动态页面的多个 GIF 帧有内容且随时间变化；原生 HWND 不截断；取消、重开和排除失败行为明确 |
| 长截图蓝色选区线进入图像；选区绘制与滚动实时抓取共存 | Antigravity / `codex/windows-scroll-overlay` | 真实滚动拼接无蓝框，保留选区全部真实像素；复用 Qoder 原生排除 |
| 截图导出只有预设目录，缺少另存路径交互 | CodeBuddy / `codex/windows-screenshot-save` | 原生路径选择、取消不写入、中文和空格路径、一键默认目录、扩展名与覆盖确认 |
| WorkBuddy 等 VS Code 类 IDE 中快捷键呼出候选窗后键盘焦点未转移 | Zed Agent / `codex/windows-clipboard-focus` | 上下键与回车操作候选项；Esc 与粘贴正确恢复原目标；失败时不误输入其他窗口 |
| 普通便笺、剪贴板预览等强制使用等宽字体，中文回退不完整 | Codex / `codex/windows-gui-fixes` | 普通文本使用无衬线；代码保留等宽并补中文无衬线回退；画布标注也一致 |

## 已有 Windows 实现，仍需专项验收

- OCR：`platform/winocr.rs` 已调用 `Windows.Media.Ocr`，缺语言包时明确报错；自定义 ONNX 尚未实现，属于独立功能。
- 划词：`platform/selection/windows.rs` 已接 UIAutomation TextPattern；`translator_mouse.rs` 有 Windows 鼠标钩子、物理坐标换算和触发按钮。提供程序不暴露选区时仍明确不支持；模拟复制兜底尚未实现，不能无条件发送 Ctrl+C 覆盖用户剪贴板。
- 时长统计：`platform/usage/windows.rs`、`windows_events.rs` 已接前台进程、GetLastInputInfo、WTS、电源事件和 Core Audio。真实锁屏/睡眠、音频播放/暂停和设备热插拔尚待用户环境验收。
- 密码本：Windows Hello NGC 设备密钥保护已实现，并已由用户确认无额外小窗、启用及解锁成功。重启后解锁、取消、提供程序/策略拒绝仍需专项；通用 `BiometricAuthenticator::verify_identity` 的 Unsupported 是刻意拒绝“认证布尔值当解钥”，不是密码本未迁移。
- 窗口置顶、物理复制粘贴、凭据库、敏感剪贴板都有 Windows 原生路径。混合 DPI/负坐标副屏、提权目标、竞争焦点、锁屏恢复不能仅靠纯逻辑单测宣称通过。
- 安装交付：目前验证调试 exe；release 安装/升级/卸载、新账户、WebView2 缺失场景仍未验收。

## 进一步发现

`src-tauri/src/translator_mouse.rs::popover_position` 目前只有 macOS 在翻译气泡尺寸改变后重新约束显示器边界。Windows 初次鼠标触发按钮已约束，气泡展开后的屏幕边缘行为仍需修正和多屏验证。此项与已实现的划词提取分开记录，不能笼统称 Windows 划词未实现。

2026-11-10 更新：Antigravity `350765e` 已补齐 Windows 工作区和 DPI 换算约束，Codex 再通过真实窗口烟测修正系统尺寸边框导致的额外越界；本机 100% DPI 的原生右下角检查通过。真实多屏/混合 DPI 与全局逻辑坐标重叠的显示器选择歧义仍未覆盖，详见 [后续分工与结果](../WINDOWS_FOLLOWUP_COORDINATION_2026-11-10.md)。

## 磁盘与工作区

四个旧 Windows worktree 已先检查分支提交全部属于 master、无未提交文件，再清理其目录；分支引用保留。旧 secure-services 的运行数据已备份至 `D:\YourAllIn1Helper-archive\windows-secure-services-2026-11-08\.runtime_data`。C 盘释放约 11.8 GiB；本轮四个独立 worktree 与构建 target 位于 D 盘，各 Cargo jobs=1，完整桌面构建由集成目录执行。
