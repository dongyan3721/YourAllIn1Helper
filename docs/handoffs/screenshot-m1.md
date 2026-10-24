# HANDOFF — 功能 01 增强截图与贴图 M1

> 状态：**M1 首个可用闭环已实现并通过自动化与原生烟测；人工 GUI 交互回归未执行**（原因见「未执行项」）。这不是功能 01 的完整交付：OCR、控件级智能选区、ONNX、长截图、GIF 均未开始。

## 基本信息

- 智能体 / 模型：Cursor Agent / Claude Opus 5.5（用户指定，全程未更换，未遇额度阻塞）。
- worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/cursor-screenshot`
- 分支：`codex/feature-screenshot`；基线 `70b607c`（包含 `f0db53d` 工具栏层级修复）。
- 未 merge、未 push；未修改主仓库工作区或其他成员目录。

| 提交 | 内容 |
| --- | --- |
| `24c3030` | os-hal 屏幕录制权限预检；module 01 两个窗口预设调整 |
| `4f89445` | 新增 `crates/mod-screenshot` 领域模块与 39 个单元/生命周期测试 |
| `588bb48` | 宿主接线：IPC、二进制 IPC、遮罩层级、贴图能力、快捷键失败处理、托盘、测试 |
| `b66909b` | 前端：遮罩、贴图、主面板截图页、三语字典、前端单测 |
| 本文提交 | 交接文档、GUI 用例、状态文档更新、构建清单 |

## 实现范围（M1）

### 截图流程与坐标

1. F1 / 托盘「区域截图」/ 主面板「开始截图」→ `ScreenCapturer::check_capture_permission` 预检（macOS `CGPreflightScreenCaptureAccess`，只检查不申请）。
2. 通过注入的 `ScreenCapturer` 抓取**真实冻结帧**；主面板按钮触发时抓帧期间临时隐藏主面板（RAII 恢复）。
3. 冻结帧完成后才创建 `capture-overlay`（宿主以 `visible(false)` 创建）；前端经二进制 IPC 取帧、绘制两帧后调用 `screenshot_overlay_ready`，宿主才显示并提升层级。桌面不会先露出空遮罩。
4. 坐标：前后端交换的选区、区域、标注全部用**帧物理像素**；`MonitorGeometry` 由「帧宽 / 桌面单位宽」与缩放因子推导换算，同时覆盖 macOS（桌面单位为点、Retina 帧为 2 倍）、Windows 高 DPI（桌面单位为物理像素）与负坐标副屏，无平台分支。后端导出前用 `normalize_crop` 校验裁剪。
5. 已有会话时再次触发以新冻结帧为准；开始截图只 `try_lock`，主线程快捷键不会等待正在等主线程建窗的 IPC。

### 智能选区（如实范围）

- **L1 窗口级**：`RegionDetector::detect_hierarchical_regions` 的顶层窗口按 HAL 返回的前→后 Z 序投影到帧像素；层级求解**复用** `app_core::resolve_smart_region_at_point`。该算法按面积取最小、不感知遮挡，因此先取光标处最前窗口，再与显示器根节点一起交给算法，避免被遮挡的小窗口穿透。滚轮 / Tab 在「窗口 ↔ 整屏」间切换。
- **L2 控件级（无障碍树）与 L3 视觉候选**：未实现，会话描述中返回 `unavailable` 及原因，主面板标注「待开发」。没有虚构窗口树或控件。
- 窗口列表获取失败时降级为手动选区，并在 `capabilities.window_tree` 中记录真实错误。

### 编辑

- 手动拖选、单击采用高亮区域、八手柄（拖过对边自动翻转）、选区内拖动、按住 Space 拖移、方向键 1px、Shift+方向键调宽高。
- 矩形、箭头、马赛克（按块对原始冻结帧取均值，非画布降采样）、序号（撤销后自动回退）、文字（Enter 确认，跳过输入法合成中的 Enter）。
- 快照式撤销/重做；放大镜 15×15 像素、WASD/方向键逐像素移动取色点，C / Shift+C 复制 `#RRGGBB` / `rgb(r, g, b)`（后端只接受这两种格式）。
- 工具栏、手柄、放大镜、文字框都是遮罩**同一 WebView 内的 DOM 层**，z-index 高于两层画布并阻止事件穿透，不存在屏幕注释那种两个同层原生窗口相互重排的问题；已有标注时点击选区外/右键不会清除编辑。

### 导出与贴图

- 前端只把选区坐标与「选区大小的透明标注层 PNG」交回；后端裁剪、Alpha 合成、PNG 编码。
- **复制**：真实系统剪贴板 RGBA 图片（宿主注入 `arboard` 适配器）。仅复制失败时整体失败，会话保留、遮罩重新显示供改为保存。
- **保存**：`create_new` 独占创建，同名追加 `_1`、`_2`…，绝不覆盖；写入失败删除本次半成品。默认目录 `应用数据/screenshots`，可配置；可选保存后同时复制（复制失败单独报告）。
- **贴图** `pin-img-{id}`：放在原截图位置、100% 时与屏幕所见同尺寸；拖动（按下移动 3px 后才交给系统拖动，保留双击）、滚轮缩放 10%–500%、Shift+滚轮透明度 10%–100%（兼容 macOS Shift 滚轮转横向）、双击折叠为 40×40 色块、复制/另存/关闭；关闭即销毁窗口并释放 PNG，系统关闭（如 ⌘W）同样释放。数量上限可配置（默认 16，最大 64）。

### 生命周期、开关、权限、快捷键

- `on_init` 只读配置，**不抓屏、不建窗口、无后台任务**。
- 停用 / 退出：先丢弃冻结帧再销毁遮罩，再关闭全部贴图；可重复调用；宿主另外兜底销毁同名残留原生窗口。停用后 JSON 与二进制入口都经 `with_enabled` 拒绝。
- 缺权限：不创建遮罩，返回带系统设置路径的 `PermissionDenied`；快捷键触发时打开主面板截图页展示指引；不自动申请或修改系统授权。
- 快捷键：M1 只声明已实现的 `screenshot:capture`（F1）；长截图、GIF、OCR 的快捷键待对应阶段再声明，不占用无效按键。核心注册表冲突时启用失败并回滚；**系统级注册失败**时，若失败项属于刚开启的模块则撤销开启并报错，其余失败记录到宿主并在截图页显示，不再因一个按键失败导致整个应用启动失败。

### 二进制 IPC 方案

| 命令 | 方向 | 载荷 |
| --- | --- | --- |
| `screenshot_frame(sessionId)` | 后端 → 前端 | 冻结帧 RGBA 原始字节（`tauri::ipc::Response` → `ArrayBuffer`），前端直接构造 `ImageData` |
| `screenshot_pin_image(pinId)` | 后端 → 前端 | 贴图 PNG 原始字节 → Blob URL |
| `screenshot_export` | 前端 → 后端 | 原始请求体为标注层 PNG（可为空）；请求头 `x-aio-screenshot-export` 为 `{session_id, selection, action}` JSON |

三个命令均为 async，复制/编码放入 `spawn_blocking`，不占 AppKit 主线程。**不是零拷贝**：后端从会话复制一次帧字节，跨进程传输与 JS 侧各还有一次复制；但没有 JSON 数组或 base64 膨胀。小数据（会话描述、区域解析、状态、设置）走原有统一 JSON `ipc`。

### 配置与升级

- 模块配置 `modules.settings.screenshot`（`config.rs`）：保存目录、文件名前缀（过滤路径分隔符）、保存后复制、默认颜色/线宽、贴图默认不透明度、贴图上限；未知字段原样保留，合并更新类型错误整体拒绝。
- 智能选区/放大镜/滚轮/手动框选开关沿用核心层已有 `AppConfig.screenshot`，与模块配置在一次 `update_config` 中原子保存。没有新增数据库迁移，旧数据不受影响。

## 公共文件清单（需 Codex 集成审查）

- `Cargo.toml`（workspace 成员）、`Cargo.lock`（仅新增本 crate 条目，无新外部依赖版本）。
- `crates/os-hal/src/traits.rs`：`ScreenCapturer::check_capture_permission` 默认方法（默认 `Ok`，Windows 无独立授权概念）。
- `crates/os-hal/src/platform/screen.rs`、`crates/os-hal/tests/capabilities.rs`。
- `crates/app-core/src/window_manager.rs`：仅改模块 01 的 `capture_overlay`（不透明）与 `pin_image`（禁止边框改尺寸、最小 16）预设。
- `src-tauri/Cargo.toml`、`src/lib.rs`（`screenshot` 字段、`native_shortcut_failures`）、`src/modules.rs`、`src/ipc/mod.rs`。
- `src-tauri/src/tauri_app.rs`：遮罩隐藏创建/显示特例；**`register_global_shortcuts` 语义变更（影响所有模块）**：单键失败继续注册其他键并返回失败列表，只回滚刚开启且自身失败的模块；托盘「区域截图」；贴图/遮罩 `CloseRequested` 释放；注册三个二进制命令；生产进程注入 `arboard` 剪贴板。
- `src-tauri/capabilities/screenshot-pins.json`：仅 `pin-img-*` 的 `core:window:allow-start-dragging`、`core:window:allow-set-size`；遮罩无任何窗口/事件权限。
- 测试：`src-tauri/tests/{modules.rs, feature_integration.rs, unit/lib.rs, support/ui_smoke.rs}` 的挂载数 6→7，`ui_smoke` 新增截图生命周期检查。
- 前端：`App.vue`、`Dashboard.vue`（01 页签与 `/screenshot` 路由）、三语字典（仅新增 `tabs.screenshot` 与 `screenshot.*`）、`package.json` 测试脚本（`--experimental-strip-types` 直接导入 TS 源码，需 Node ≥ 22.6）。
- 文档：`docs/MODULE_ARCHITECTURE.md`、`DEVELOPMENT_CHECKLIST.md` 01 行、`docs/development/REMAINING_FEATURES_PLAN_2026-10-25.md` 状态列。

模块自有新文件：`crates/mod-screenshot/**`、`src-tauri/src/{ipc/screenshot.rs, screenshot_binary.rs, screenshot_clipboard.rs, screenshot_window.rs}`、`src-tauri/tests/screenshot.rs`、`src-ui/src/{api/screenshot.ts, components/CaptureOverlay.vue, components/PinImage.vue, components/ScreenshotPanel.vue, utils/screenshot/*}`、`src-ui/tests/unit/screenshot.test.mjs`、`docs/testing/GUI_TEST_SCREENSHOT.md`。

## 实际执行的测试（macOS 27.0，arm64）

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `CARGO_TARGET_DIR=$PWD/target cargo test --workspace --offline --all-targets` | 通过：119 passed，0 failed，1 ignored（既有 `test_native_windows_list`） |
| `cargo test -p mod-screenshot --offline` | 通过：单元 25 + 生命周期 14 |
| `cargo test -p all-in-one-host --test screenshot --offline` | 通过：3（JSON/二进制全流程、停用拒绝与释放、配置原子保存） |
| `cargo test --manifest-path templates/module/Cargo.toml --offline` | 通过 |
| `pnpm --dir src-ui run build`（`vue-tsc --noEmit && vite build`） | 通过（仅既有 chunk 体积提示） |
| `pnpm --dir src-ui test` | 通过：16（其中截图 12） |
| `bash scripts/package_macos_app.sh` | 通过：release `.app`，未签名 |
| `target/release/all-in-one-desktop --ui-smoke-test` | **PASS**；屏幕录制已授权，截图分支完整执行：真实抓帧 → 遮罩隐藏 → 帧字节长度 → `overlay_ready` 后可见 → **原生层级 > `NSMainMenuWindowLevel`** → 贴图替换遮罩 → 关闭贴图销毁 → 停用销毁遮罩并释放冻结帧 |
| `python3 scripts/prepare_gui_test_app.py` | 通过：QA 副本与 `docs/testing/build-manifest-2026-10-25.json`；可执行文件 SHA-256 `2605f1ae77b5b962bd445ba59ff1ad04e51ba56a3715a8c17e1202f41ccb9e17` |

单元/集成测试覆盖任务书要求：裁剪/缩放/负坐标边界、选区几何与序列化、编辑操作（前端历史/序号/马赛克）、文件不覆盖、停用拒绝 IPC（含二进制入口）、初始化不抓屏、取消释放帧、关闭销毁、配置与重启保留（含未知字段）、快捷键冲突回滚。测试只使用自制渐变图与临时目录；无桌面测试注入「不可用」剪贴板，不写系统剪贴板。

环境说明：沙箱内 `libsqlite3-sys` 构建脚本无法从 `~/.cargo/registry` 复制文件，Rust 构建与测试在沙箱外执行，`CARGO_TARGET_DIR` 指向本 worktree 的 `target/`；离线构建，无下载。前端依赖用 `pnpm install --frozen-lockfile` 装入本目录 `src-ui/node_modules`，61 个包全部来自本机 pnpm 内容存储，0 下载，锁文件未变。

## 未执行项与平台限制

- **人工 GUI 交互回归全部未执行**：按 GUI 测试计划启动 QA App 时显示器处于屏保/锁定状态，Computer Use 截图为系统屏保画面，App 窗口被判定为遮挡、WebView 不绘制。没有尝试解锁或绕过系统安全设置。用例与状态见 [GUI_TEST_SCREENSHOT.md](../testing/GUI_TEST_SCREENSHOT.md)，关键项：Retina 裁剪尺寸、标注后工具栏可点击、贴图缩放/透明度/折叠、Esc/停用/重启无残留、权限拒绝。
- 原生烟测运行时屏幕同样处于屏保状态：它验证了窗口生命周期、可见性与原生层级，**不代表冻结帧视觉内容已人工核对**。
- 实体键盘 F1 未验证。MacBook 默认 F1 是亮度键，需 fn+F1 或在系统中开启「将 F1、F2 等键用作标准功能键」；当前主面板没有快捷键改键界面，只能通过 `config.json` 的 `shortcut_overrides["screenshot:capture"]` 修改。
- 输入法候选窗与遮罩层级（遮罩为菜单栏层 +2）未实测。
- 多显示器：每次只冻结一块显示器（F1 为主屏，主面板可选择显示器）；未实现跨屏同时冻结与跨屏选区。
- Windows：编译期分支未在 Windows 构建或实机运行；混合 DPI 下遮罩逻辑坐标换算未验证。
- 已创建的标注不能再次选中移动/修改，只能撤销重做；无「另存为」系统对话框（现有原生对话框只有打开面板）。

## 后续待办（未实现，不得视为完成）

- **M2**：macOS Vision / Windows.Media.Ocr 真实 OCR 与坐标；无障碍控件级（L2）与视觉候选（L3）智能选区及滚轮/Tab 父子层级；自定义 ONNX 模型、执行提供器探测与回退、180 秒闲置释放；`screenshot:ocr` 快捷键。
- **M3**：NCC 滚动长截图（重复帧、固定头尾、低置信度、取消、长度/内存上限）与 GIF 录制编码；`screenshot:scroll` / `screenshot:gif` 快捷键。
- 人工 GUI 回归、实体 F1、多显示器、Windows 实机验收。
