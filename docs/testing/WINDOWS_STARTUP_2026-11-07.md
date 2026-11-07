# Windows 首次环境搭建与启动验证（2026-11-07）

本轮目标是补齐本机开发环境、消除阻止 Windows 构建和启动的问题，并验证真实桌面宿主。不是十个模块的完整 Windows 发布验收。

## 环境

- Windows x64，25H2，Build 26200（注册表的 ProductName 仍显示 Windows 10，不能据此判断实际系统代际）。
- Node.js 24.17.0；npm 11.13.0；pnpm 10.34.9。
- Rust / Cargo 1.99.1，`stable-x86_64-pc-windows-msvc`，minimal profile。
- Visual Studio 2022 Build Tools 17.14.44，MSVC v143 14.44，Windows SDK 10.0.26100.0。
- 已有 WebView2 Runtime 156.0.4314.58。

Rust、Visual C++ 构建工具为本轮新增。系统原有 pnpm shim 指向自身，已备份为 `%LOCALAPPDATA%\pnpm\bin\pnpm.exe.bak`，改用 npm 全局安装的 pnpm。Rust 安装器已将 `%USERPROFILE%\.cargo\bin` 加入用户 PATH；已打开的 IDE 需重启才能继承。

## 修复

1. 根目录脚本引用了不存在的 `@tauri-apps/cli/bin/tauri.js`，改为已安装包实际提供的 `@tauri-apps/cli/tauri.js`。
2. `build:app` 原来强制使用 macOS 专属的 `--bundles app`；改为当前平台默认打包目标，增加统一 `dev`、`setup` 和无安装包的 `build:desktop` 命令。
3. Win32 `SetWindowPos` 的 Z 序句柄改为显式 `Some(HWND)`，匹配锁定的 windows-rs 0.62 API。
4. `OpenInputDesktop` 的空标志改为 `DESKTOP_CONTROL_FLAGS(0)`，匹配强类型 API。
5. 新增根目录 README，说明结构、Windows 前置依赖、启动、构建和测试命令。
6. Batch 解释器的 PATH 回退查询改为通过 `EnvProbeService` 类型调用，修复 Windows 条件分支编译错误。
7. 外部工具探测检查退出码，不再把退出 9009 的 Windows Python 执行别名标成可用；本机可回退到实际安装的 `C:\SDK\Python313\python.exe`。增加成功和失败探测的子进程回归测试。
8. PowerShell 版本探测改为兼容 Windows PowerShell 5.1 与 PowerShell 7 的无配置文件查询。
9. 原生 UI 烟测新增便笺就绪握手：Windows WebView 创建返回时，页面可能还没订阅保存事件；先确认真实保存握手可响应，再停用模块。没有修改生产保存协议或缩减保存检查。

未升级 Cargo.lock 或 src-ui/pnpm-lock.yaml 中的依赖。

## 验证记录

| 检查 | 结果 |
| --- | --- |
| `pnpm run setup` | 按前端锁文件安装成功 |
| `pnpm tauri info` | MSVC、Rust、Cargo、WebView2 环境项全部通过 |
| `pnpm --dir src-ui run build` | TypeScript 检查及生产构建通过 |
| `pnpm --dir src-ui test` | 73 项通过 |
| `cargo test --workspace --locked --all-targets` | 32 个测试程序，253 项通过、0 失败、1 项显式忽略 |
| `pnpm build:desktop` | Windows x64 调试版构建成功，嵌入界面资源，无需 Vite 服务 |
| `all-in-one-desktop.exe --smoke-test` | 5 步无桌面 IPC 烟测通过，使用显式 Mock OS HAL |
| `all-in-one-desktop.exe --ui-smoke-test` | 真实 Windows 窗口、截图冻结帧与贴图、模块启停、便笺保存/销毁/恢复、屏幕注释窗口、快捷键注册与清理通过；退出码 0 |
| 实际 GUI 交互 | 主控台、十模块入口、截图页、便笺库切换正常；新建便笺、设置中英文正文、Markdown/LaTeX 预览、保存、关闭归档、唤醒后文本保留均通过 |
| 应用退出及重启 | 通过界面退出；用同一隔离目录重启成功，归档便笺记录保留；再次正常退出 |
| 最终启动 | 已启动 `target/debug/all-in-one-desktop.exe` 的普通实例，未设置隔离数据目录 |

本机已有 FFmpeg / FFprobe；Rust 测试中的真实 FFmpeg 集成用例通过。Python 和 Node 的真实脚本执行用例通过；不受 Windows 支持的 Shell/AppleScript 用例有提前返回逻辑，不能算成这些解释器在 Windows 上通过验收。被忽略项是 `os-hal` 的原生窗口枚举测试。`native_ocr` 专项用例受 macOS 条件编译限制，在 Windows 上没有执行。

首次原生 UI 烟测在便笺刚创建后立即停用时遇到保存事件尚未订阅的竞态；补充真实页面就绪握手后复测通过。生产代码仍保留超时不销毁窗口的保存保护。

UI 烟测批量销毁窗口时，底层输出了 `PostMessage ... 无效的窗口句柄 (0x80070578)` 诊断，烟测结果为 PASS 且进程正常退出；普通 GUI 退出日志没有复现。构建仍有平台分支的未使用项警告，以及 Vite 大分块警告，本轮没有扩大修改范围处理这些告警。

## 构建与证据

- 基础提交：`5df7291b509f246cf697ef722dd36747bcc16001`，包含本轮未提交修改。
- 可执行文件：`target/debug/all-in-one-desktop.exe`，43,993,600 字节。
- SHA-256：`81991035ACE05FA2B4C2277D35C161F080FAFC886FF739064EC56874807F5138`。
- GUI 测试副本：`.runtime_data/windows-validation/app/all-in-one-desktop.exe`，与上述文件哈希一致。
- 人工测试数据：`.runtime_data/windows-validation/manual`；仅创建虚构便笺，没有改动默认用户数据。
- 本机完整构建、环境、测试和烟测日志：`.runtime_data/windows-validation/`（忽略提交）。
- 截图证据：[主控台](evidence/windows-startup-2026-11-07/overview.png)、[截图设置页](evidence/windows-startup-2026-11-07/screenshot-panel.png)、[便笺预览](evidence/windows-startup-2026-11-07/sticky-preview.png)、[重新唤醒](evidence/windows-startup-2026-11-07/sticky-reopened.png)、[重启后便笺库](evidence/windows-startup-2026-11-07/restart-library.png)。

## 已知边界

- Windows 系统凭据存储尚未接入，翻译密钥只在会话内有效。
- Windows 敏感剪贴板所有权和动态 GIF 剪贴板写入尚未接入。
- Windows Hello / 生物识别解钥尚未接入；macOS Touch ID 不适用于 Windows。
- Windows 原生睡眠/会话事件观察和默认音频输出活动探测尚未接入。
- FFmpeg / FFprobe、翻译服务、不同脚本解释器属于可选运行依赖；本轮未逐项完成人工 GUI 功能验收。
- 部分界面说明仍沿用 macOS 文案（如便笺快捷键和 OCR 平台说明），本轮未逐页校正。
- Unix 专用测试以及真实 OCR、全局输入、跨应用粘贴、外部窗口置顶、多屏 DPI 等需单独记录 Windows 专项结果，不能由编译成功推断为通过。
