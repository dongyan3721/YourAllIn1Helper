# Windows GUI 修复第二轮分工（2026-11-08）

集成负责人 Codex；基线 `0cdada8`。用户确认的四项缺陷是 GIF 黑屏、长截图蓝框入图、截图保存缺少路径选择、Windows 中文字体回退。额外处理已知的 VS Code 类 IDE 剪贴板候选窗焦点缺口。

| 智能体 | 分支 | 独立工作目录 | 任务 |
| --- | --- | --- | --- |
| Qoder / deepseek-flash | `codex/windows-gif-capture` | `D:\YourAllIn1Helper-worktrees\gif` | GIF 黑屏与 Windows 实时捕获的原生遮罩排除 |
| Antigravity / 保留当前 Gemini | `codex/windows-scroll-overlay` | `D:\YourAllIn1Helper-worktrees\scroll` | 长截图蓝框污染、滚轮透传及边界 |
| CodeBuddy / 保留当前 MiMo | `codex/windows-screenshot-save` | `D:\YourAllIn1Helper-worktrees\save` | 截图选择保存路径与一键默认保存 |
| Zed Agent / Sonnet 5.5 | `codex/windows-clipboard-focus` | `D:\YourAllIn1Helper-worktrees\focus` | VS Code 类 IDE 呼出候选窗焦点、上下键选择 |
| Codex | `codex/windows-gui-fixes` | 主工作目录 | Windows 无衬线字体、范围审计、共享接口协调、代码审查、集成与 GUI 验收 |

## 文件职责与协作接口

- Qoder 主责 `crates/os-hal/src/platform/screen.rs`、必要的 Windows 捕获子实现、`src-tauri/src/screenshot_window.rs`、GIF recorder/encoder 与其测试。修复 Windows 实时捕获时登记有效遮罩标识/排除，而不是用伪造帧或冻结截图冒充录制。`capture_region_below` 当前仅 macOS 真正排除遮罩；该原生排除能力由 Qoder 独占提供，Antigravity 复用，不另写竞争实现。Win64 HWND 不得静默截断；如改共享标识类型，列明所有调用点，由 Codex 处理交叉集成。
- Antigravity 主责 `src-ui/src/components/CaptureOverlay.vue`、`src-ui/src/utils/screenshot/scroll.ts`、`crates/mod-screenshot/src/scroll/**` 及相应测试。蓝框污染应从绘制/捕获边界修复，不能裁去用户真实选择区的像素来掩盖。GIF UI 若须修改 CaptureOverlay，由 Antigravity 协调自己的改动；Qoder 在报告中提供所需接口，不同时改此文件。
- CodeBuddy 主责截图保存接口、`src-tauri/src/native_dialog.rs`（或新建专用 save dialog 模块）、`src-tauri/src/ipc/screenshot.rs` 的导出路径、`src-ui/src/components/ScreenshotPreview.vue`、截图 API 与三语保存文案/测试。提供“选择路径保存”和“一键默认路径保存”，明确取消、不覆盖或覆盖确认、中文/空格路径、PNG/GIF 扩展名。桌面原生保存对话框保持异步，不阻塞 UI；不得 shell 拼接执行用户路径。`mod-screenshot/src/lib.rs` 只改导出/保存相关函数。
- Zed 主责 `src-tauri/src/clipboard_window.rs`、Windows 前台激活/候选窗展示的必要原生代码、ClipboardPopup 的脚本和键盘处理及测试。不得绕过 Windows 前台限制或无条件向任意窗口发送输入。`src-tauri/src/tauri_app.rs` 如需修改，只改 popup 创建/聚焦代码；截图导出分派由 CodeBuddy 改，其他共享冲突交给 Codex。不得改 ClipboardPopup 的字体/CSS。
- Codex 主责全局字体与 StickyNote/ClipboardPopup 的字体 CSS，保留代码和数学排版的有意等宽/数学字体。跨模块共享入口（截图 lib.rs、Tauri IPC 分派、Cargo Windows feature）只能增量改自己任务的函数，禁止全文件格式化或重写他人区域；报告中列出依赖，Codex 统一处理。

## 执行要求

1. 只在分配的 D 盘 worktree 内操作、提交，不在主目录切换分支或修改文件。先读 README、模块架构和此文；旧迁移报告是历史状态，以代码和真实结果为准。
2. 每个 worktree 的 `target` 在 D 盘，Cargo jobs=1；不要创建 C 盘的新 target。Rust PATH 如缺失，前置 `%USERPROFILE%\.cargo\bin`。只运行相关 crate/前端的必要测试，避免同时四份 Tauri 全工作区构建；最终完整构建由 Codex 负责。
3. 不升级依赖、不重写 Cargo.lock；需要 Windows feature 时只加必要条目并说明。保留 macOS 条件编译行为。生产使用真实系统能力，不能以 Mock、固定成功/空帧冒充修复。
4. 有意义的单测覆盖此次缺陷、失败与取消、尺寸/DPI边界和清理。普通测试不读取用户秘密、不改真实剪贴板、不弹认证；原生 GUI/硬件专项单独 ignored 并如实注明。没有运行的测试不能记作通过。
5. **不要启动桌面应用、抢焦点、发送全局快捷键或做 GUI 自动化**：本轮仅 Codex 集中进行真实桌面验收，防止多个智能体互相干扰。使用虚构测试数据。
6. 不合并、不推送、不删除其他分支/worktree。完成后提交自己修改，新增 `docs/testing/WINDOWS_GUI_<GIF|SCROLL|SAVE|FOCUS>_2026-11-08.md`，记录根因、实现、测试命令/结果、已知边界和需集成的共享改动；在当前会话回复提交哈希。

## Windows 未覆盖项的审计边界

已经实现但仍需硬件/兼容验收的项包括锁屏/睡眠/会话切换、音频热插拔、混合 DPI/负坐标副屏、Hello 的取消/重启/策略缺失、GIF 在各目标应用的动画粘贴，以及 release 安装包。不能把这些统一称为“尚未迁移”。本轮优先修复有可复现缺陷或源码明确缺失的 Windows 路径；新增发现另列证据，不擅自扩大到未配置的网络服务或系统隐私设置。
