# 功能 03：桌面屏幕注释（Screen Ink）交接报告 (HANDOFF.md)

## 1. 分支与提交基本信息

- **认领智能体**：Antigravity (Gemini 3.8 Flash)
- **负责功能**：子功能 03：桌面屏幕注释 (Screen Ink)
- **工作目录**：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/antigravity-screen-ink`
- **开发分支**：`codex/feature-screen-ink`
- **基线 Commit**：`68fecea`
- **首次交付 Commit**：`c4ebd6c`
- **审查修复 Commit**：待本次提交生成 (fix commit, 不 amend 原提交)
- **Git 状态**：严格在独立 worktree 工作，未触碰主目录或 master 分支，未执行 merge/push，等待 Codex 提取合入。

---

## 2. Codex 审查发现逐项修复清单

根据 Codex 审查提出的 6 项改进要求，已全部完成实质修复与针对性测试补齐：

### 2.1 窗口生命周期原子回滚与真实状态一致（审查项 1）
- **第二窗口启动失败回滚**：在 `start_session` 中，若创建 `screen-ink-toolbar` 失败，立即调用 `destroy_window("screen-ink-canvas")` 回滚已创建的全屏画布，阻止孤儿窗口残留，`is_active` 保持 `false`。
- **销毁错误真实传递**：在 `exit_session` 与 `on_shutdown` 中，严格检查画布与工具栏真实销毁结果，若系统原生销毁报错或窗口仍处于活跃窗口池中，准确返回 `Err`，`is_active` 保持为 `true`，拒绝谎报已关闭，支持故障恢复后重试。
- **重复启动穿透状态重置**：当模块已启动但处于穿透状态时，再次调用 `start_session` 不仅将内存状态重置为 `is_passthrough = false`，同时显式调用 `window_pool.set_mouse_passthrough("screen-ink-canvas", false)`，保证原生窗口与状态一致。
- **自动化测试**：在 `tests/lifecycle.rs` 中新增 `test_screen_ink_start_session_rollback_on_failure` 与 `test_screen_ink_exit_session_error_handling_and_retry`，在 `tests/unit/lib.rs` 中新增 `test_screen_ink_repeated_start_resets_passthrough`。

### 2.2 事件桥接打通与时序保护（审查项 2）
- **Native 事件桥接**：在 `tauri_app.rs` 中将 `ink_clear_canvas`、`ink_set_passthrough`、`ink_start_session`、`ink_exit_session` 以及快捷键触发联动发布为 Tauri 事件 `screen-ink://mode-changed` 与 `screen-ink://clear-canvas`。
- **前端双向订阅与卸载清理**：`ScreenInkToolbar.vue` 与 `ScreenInkCanvas.vue` 均在 `onMounted` 中通过 `listen(...)` 订阅原生事件，并在 `onUnmounted` 中彻底注销监听函数。
- **时序状态补偿**：工具栏在挂载后主动向画布发送 `request_state_sync` 信号，画布收到后立即通过 `notifyToolbar()` 回传完整画笔、颜色、线宽和模式状态，彻底解决首次加载顺序不确定导致的 `state_sync` 丢失。

### 2.3 工具栏尺寸自适应与弹层无裁切（审查项 3）
- **窗口规格升级**：在 `crates/app-core/src/window_manager.rs` 中将工具栏窗口尺寸由 440x56 调整为 580x130，水平居中（`(screen_w - 580) / 2`）。
- **布局优化**：WebView 内部保持紧凑的 42px 毛玻璃胶囊栏，内部容纳 11 个核心按钮；上方预留充裕空间供颜色/线宽选择浮层弹出，下方浮动提示 Toast，彻底杜绝 WebView 边界剪切。

### 2.4 真实单帧干净底图捕获（审查项 4）
- **临时隐藏窗口与 RAII 守护**：实现 `CleanCaptureVisibilityGuard` 守护结构体，在调用 `ScreenCapturer::capture_monitor` 前，将 `screen-ink-canvas` 与 `screen-ink-toolbar` 临时隐藏（`set_window_visible(false)`），并留出 40ms 刷新间隙以供系统窗口合成器剔除自身内容。
- **防死锁与可靠恢复**：RAII 守护在离开作用域时无论成功或报错均无条件恢复窗口显示（`set_window_visible(true)`），杜绝因异常导致窗口永久失联。
- **无双重叠墨**：合成时背景为纯净系统底图，上方仅叠加当前透明画布单层墨迹，彻底解决将墨迹与工具栏重复截入快照的问题。

### 2.5 配置默认值、规范清洗、独占文件保存与剪贴板准确反馈（审查项 5）
- **配置健壮性**：`ScreenInkConfig` 增加 `#[serde(default)]`，提供 `sanitize()` 方法对工具合法性、十六进制色值格式、线宽范围（1.0~50.0px）与目录合法性进行校验与容错清洗；在 `on_init` 时自动应用配置到初始状态。
- **独占创建防覆盖**：`save_png_exclusive` 采用 `std::fs::OpenOptions::new().create_new(true)` 独占模式，同秒或文件名冲突时自动递增 `_1.png`、`_2.png` 序号重试，绝不覆盖已有文件。
- **剪贴板真实反馈**：`composite_and_save` 返回 `clipboard_error: Option<String>`；前端在文件保存成功但剪贴板写入失败时，准确显示多语言 Toast `toastSaveFileOnly`（"快照已保存至本地，但复制到剪贴板失败"），三语言字典全量对齐。

### 2.6 消除 mock_bg_frame 与去除写死白图（审查项 6）
- **接口提纯**：从 `ScreenInkModule::composite_and_save` 及 IPC `ink_save_snapshot` 参数列表中彻底移除 `mock_bg_frame`。
- **真实 HAL 驱动**：生产环境底图抓取完全依赖注入的 `ScreenCapturer` 实现；测试环境通过 `MockOsHal` 提供受控单帧，捕获失败直接向调用栈返回错误，严禁回退伪造纯白假图。

---

## 3. 测试命令及真实结果

所有测试均在本地独立 worktree（`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/antigravity-screen-ink`）执行：

| 测试命令 | 测试目标 | 真实执行结果 |
| :--- | :--- | :--- |
| `CARGO_TARGET_DIR=... cargo test -p mod-screen-ink --offline` | `mod-screen-ink` 专属单元与生命周期测试（共 11 项） | **全部通过** (11 passed; 0 failed) |
| `CARGO_TARGET_DIR=... cargo test --workspace --offline --all-targets` | 全工作区 Rust 单元测试、生命周期测试与 Host IPC 集成测试 | **全部通过** (28 passed; 0 failed; 1 ignored[需物理会话的窗口枚举]) |
| `cargo fmt --all -- --check` | Rust 代码格式规范校验 | **通过** (Exit code 0, 无格式差异) |
| `pnpm --dir src-ui run build` (`vue-tsc --noEmit && vite build`) | 前端 TypeScript 类型检查与 Vite 生产打包 | **通过** (57 modules transformed, built in 1.36s) |
| `pnpm --dir src-ui test` | 前端多语言同构、SVG Logo 几何完整性、Ink 历史栈与激光笔消隐算法测试 | **全部通过** (4 passed; 0 failed) |

---

## 4. GUI 测试用例状态

编制并更新专属用例文档：[`docs/testing/GUI_TEST_SCREEN_INK.md`](docs/testing/GUI_TEST_SCREEN_INK.md)。

- **用例总数**：17 项（涵盖基础交互 14 项 + 审查项针对性 GUI 用例 3 项）；
- **当前执行状态**：全部标为 **「未执行（无物理桌面会话 / 待集成阶段手工回归）」**。如实标定，不将单元测试伪造为 GUI 已验证。

---

## 5. 修改公共文件列表 (用于 Codex 最终集成比对)

1. `crates/app-core/src/window_manager.rs`：新增 `set_window_visible` 接口（默认实现及 `WindowPoolManager` 转发）；将工具栏窗口尺寸调整为 580x130。
2. `src-tauri/src/ipc/screen_ink.rs`：移除 `mock_bg_frame`；新增 `ink_update_settings` 配置写入路由。
3. `src-tauri/src/tauri_app.rs`：实现 `TauriWindowBackend::set_window_visible`；打通原生快捷键/IPC 至 Tauri 事件桥；在托盘和快捷键处广播 `screen-ink://mode-changed`。
4. `src-ui/src/components/ScreenInkCanvas.vue`：支持 `request_state_sync`、`screen-ink://clear-canvas`、`screen-ink://mode-changed` 原生事件订阅及卸载清理。
5. `src-ui/src/components/ScreenInkToolbar.vue`：适配 580x130 窗口与弹层无裁切布局；主动发送 `request_state_sync`；处理 `toastSaveFileOnly` 精确反馈。
6. `src-ui/src/locales/zh-CN.json`、`en-US.json`、`ja-JP.json`：新增 `toastSaveFileOnly` 词条。
