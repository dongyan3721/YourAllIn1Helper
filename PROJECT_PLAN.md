# 全能桌面效率工具 (All-in-One Desktop App) 架构设计与开发总览规划

> **项目定位**：基于 **[Tauri 2.x](https://v2.tauri.app/) (Rust + Web)** 构建的跨平台（优先适配 **Windows & macOS**）、极低内存占用、插件化高可维护的桌面效率“瑞士军刀”。通过**微内核常驻守护进程（Core Daemon）+ 按需瞬态视图（On-Demand WebView）+ 零运行时打包（系统环境变量探测 + 原生系统 API 优先）**的架构设计，将日常办公的 10 个高频独立工具整合进同一套工程底座。

## 目录

1. [总体设计原则与核心指标](#1-总体设计原则与核心指标)
2. [主程序微内核架构与软件工程设计模式](#2-主程序微内核架构与软件工程设计模式)
   - 2.1 [整体分层架构图](#21-整体分层架构图)
   - 2.2 [推荐代码目录结构 (Cargo Workspace + Multi-Window UI)](#22-推荐代码目录结构-cargo-workspace--multi-window-ui)
   - 2.3 [核心设计模式落地](#23-核心设计模式落地)
   - 2.4 [极致省内存策略：WebView 生命周期管理](#24-极致省内存策略webview-生命周期管理)
3. [主程序公共基础设施设计](#3-主程序公共基础设施设计)
   - 3.1 [插件化模块接入标准 (`AppModule` Trait 与主程序注册表)](#31-插件化模块接入标准-appmodule-trait-与主程序注册表)
   - 3.2 [跨系统快捷键归一化与全局输入钩子](#32-跨系统快捷键归一化与全局输入钩子)
   - 3.3 [零打包运行时探测器 (`EnvProbeService`)](#33-零打包运行时探测器-envprobeservice)
   - 3.4 [本地 SQLite 存储架构与跨版本平滑升级方案](#34-本地-sqlite-存储架构与跨版本平滑升级方案)
4. [10 大子功能主程序接入点全景矩阵](#4-10-大子功能主程序接入点全景矩阵)
5. [10 大子功能模块深度拆解与主程序接入点规范](#5-10-大子功能模块深度拆解与主程序接入点规范)
   - 5.1 [子功能 01：增强截图工具 (基础截图 / 注释 / 贴图 / OCR / 长截图 / GIF 录制)](#51-子功能-01增强截图工具-基础截图--注释--贴图--ocr--长截图--gif-录制)
   - 5.2 [子功能 02：剪切板管理器 (类 Ditto)](#52-子功能-02剪切板管理器-类-ditto)
   - 5.3 [子功能 03：桌面屏幕注释 (类 gInk)](#53-子功能-03桌面屏幕注释-类-gink)
   - 5.4 [子功能 04：桌面置顶富文本便笺 (Markdown + LaTeX)](#54-子功能-04桌面置顶富文本便笺-markdown--latex)
   - 5.5 [子功能 05：应用使用时长统计 (类 Windows TAI)](#55-子功能-05应用使用时长统计-类-windows-tai)
   - 5.6 [子功能 06：自定义脚本收录与一键执行](#56-子功能-06自定义脚本收录与一键执行)
   - 5.7 [子功能 07：全局划词翻译](#57-子功能-07全局划词翻译)
   - 5.8 [子功能 08：基础多媒体处理 (图片/音视频转换与裁剪)](#58-子功能-08基础多媒体处理-图片音视频转换与裁剪)
   - 5.9 [子功能 09：任意窗口置顶小工具 (类 DeskPins)](#59-子功能-09任意窗口置顶小工具-类-deskpins)
   - 5.10 [子功能 10：生物识别加密密码本](#510-子功能-10生物识别加密密码本)
6. [分阶段开发路线图 (Roadmap)](#6-分阶段开发路线图-roadmap)

## 1. 总体设计原则与核心指标

| 维度 | 设计目标与硬性指标 | 关键技术决策 |
| :--- | :--- | :--- |
| **目标平台** | 首发完美适配 **Windows 10/11** 与 **macOS 13+**，预留 Linux 扩展接口 | 通过 Rust 条件编译 (`#[cfg(target_os = "windows")]` / `#[cfg(target_os = "macos")]`) 实现系统硬件抽象层（OS HAL） |
| **核心框架** | [Tauri 2.x](https://v2.tauri.app/) (Rust 后端 + TypeScript/Vue 3 或 React 前端) | 后台监控与系统调用 100% 运行在 Rust 层，前端仅作为按需渲染的薄视图层 |
| **内存控制** | 后台静默常驻内存 **< 25 MB**；单功能面板唤醒增量 **< 60 MB** | **零常驻隐藏 WebView**：程序启动仅驻留 Rust 守护进程与系统托盘，所有 UI 窗口按需创建、关闭即销毁释放 WebView2/WKWebView 进程 |
| **安装包体积** | 裸包安装体积 **< 15 MB** | **零运行时打包**：优先调用系统原生 OCR API，自定义模型采用 [ONNX Runtime (`ort`)](https://github.com/pykeio/ort) 动态加载用户权重；FFmpeg / Python / Node / 脚本解释器通过系统环境变量 `PATH` 探测调用 |
| **数据与升级** | 单一本地 SQLite 数据库 (`app.db`) + 版本化配置文件 (`config.json`) | 基于 [`rusqlite`](https://github.com/rusqlite/rusqlite) + [`rusqlite_migration`](https://github.com/cljoly/rusqlite_migration) 实现数据库 Schema 自动增量迁移；配置采用 `serde(default)` + 链式 `Migrator` 保证跨版本无缝升级 |

## 2. 主程序微内核架构与软件工程设计模式

### 2.1 整体分层架构图

```text
+---------------------------------------------------------------------------------------------------+
|                                  按需唤醒视图层 (On-Demand WebView UI)                             |
|  [截图/长截图画布]  [贴图浮动窗]  [屏幕墨水画板]  [剪贴板弹窗]  [置顶便笺窗]  [划词翻译气泡]  [主控台]  |
+--------------------------------------------------^------------------------------------------------+
                                                   | Tauri IPC (Commands / Typed Events / Channels)
+--------------------------------------------------v------------------------------------------------+
|                                 主程序微内核 (Tauri 2.x Rust Core Daemon)                          |
|  +-----------------------+  +------------------------+  +--------------------------------------+  |
|  | ModuleRegistry        |  | WindowPoolManager      |  | InternalEventBus (tokio::broadcast)  |  |
|  | (生命周期与插件容器)   |  | (WebView 按需创建与销毁)|  | (解耦钩子、剪贴板、计时器与各业务模块)|  |
|  +-----------------------+  +------------------------+  +--------------------------------------+  |
|  +-----------------------+  +------------------------+  +--------------------------------------+  |
|  | ShortcutNormalizer    |  | EnvProbeService        |  | StorageEngine (SQLite + ConfigMigrator)|
|  | (跨平台快捷键映射)     |  | (PATH探测:FFmpeg/Py/JS)|  | (rusqlite + AES-256-GCM 加密存储)    |  |
|  +-----------------------+  +------------------------+  +--------------------------------------+  |
+--------------------------------------------------^------------------------------------------------+
                                                   | 统一插件接口 (Trait: AppModule)
+--------------------------------------------------v------------------------------------------------+
|                                    10 大业务模块层 (Feature Modules)                               |
|  [01.Screenshot] [02.Clipboard] [03.ScreenInk] [04.StickyNote] [05.AppUsage]                      |
|  [06.ScriptRun]  [07.Translate] [08.MediaProc] [09.WindowPin]  [10.PasswordVault]                 |
+--------------------------------------------------^------------------------------------------------+
                                                   | 跨平台系统抽象层接口 (Trait: OsPlatformAdapter)
+--------------------------------------------------v------------------------------------------------+
|                                 系统硬件抽象层 (OS HAL / Native APIs)                              |
|  +-----------------------------------------------+---------------------------------------------+  |
|  | Windows 实现 (microsoft/windows-rs)           | macOS 实现 (madsmtm/objc2 & CoreFrameworks) |  |
|  | - GraphicsCapture / DXGI / PrintWindow        | - ScreenCaptureKit / CGWindowListCreateImage|  |
|  | - Windows.Media.Ocr + ONNX DirectML/CUDA/CPU  | - Vision (VNRecognizeTextRequest) + CoreML  |  |
|  | - SetWindowPos(HWND_TOPMOST) / WinEventHook   | - AXUIElement / CGWindowLevel / NSWorkspace |  |
|  | - Windows Hello (UserConsentVerifier) + DPAPI | - LocalAuthentication (Touch ID) + Keychain |  |
|  | - UIAutomation + SendInput (Ctrl+C 降级)      | - AXUIElementCopyAttributeValue + CGEvent   |  |
|  +-----------------------------------------------+---------------------------------------------+  |
+---------------------------------------------------------------------------------------------------+
```

### 2.2 推荐代码目录结构 (Cargo Workspace + Multi-Window UI)

采用 **Cargo Workspace 多 Crate 模块化结构**，强制物理隔离核心底座、系统抽象层与 10 个业务子功能模块：

```text
your_all_in_one/
├── Cargo.toml                      # Workspace 根配置
├── crates/
│   ├── app-core/                   # 微内核基础设施：AppModule Trait、EventBus、SQLite 迁移、配置升级、PATH 探测
│   ├── os-hal/                     # 跨平台系统抽象层：封装 Windows (windows-rs) 与 macOS (objc2) 原生 API
│   │   ├── src/windows/            # Win32 / WinRT 平台专用实现
│   │   └── src/macos/              # AppKit / Vision / AXUIElement / LocalAuthentication 专用实现
│   ├── mod-screenshot/             # 01. 截图、贴图、OCR (系统+ONNX降级)、长截图拼接、GIF 录制
│   ├── mod-clipboard/              # 02. 剪贴板监听与 LRU-100 历史管理
│   ├── mod-screen-ink/             # 03. 桌面墨水注释后端状态与快捷键穿透控制
│   ├── mod-sticky-note/            # 04. 便笺持久化与多窗口置顶状态同步
│   ├── mod-app-usage/              # 05. 前台窗口驻留监听、息屏检测与使用时长聚合
│   ├── mod-script-runner/          # 06. 跨系统脚本引擎探测与子进程流式执行
│   ├── mod-translator/             # 07. 全局划词捕获与多源翻译 API 网关
│   ├── mod-media-proc/             # 08. 原生图像转换 + 系统 FFmpeg 探测与任务队列
│   ├── mod-window-pin/             # 09. 外部窗口句柄拾取与置顶状态守护
│   └── mod-password-vault/         # 10. 生物识别鉴权、Argon2id/AES-256-GCM 加解密与密码表管理
├── src-tauri/                      # Tauri 2.x 主程序入口，负责组装并挂载 10 个模块 Crate
│   ├── tauri.conf.json
│   └── src/main.rs
└── src-ui/                         # 前端多窗口视图工程 (Vite + Vue 3 / React + TypeScript)
    ├── windows/
    │   ├── dashboard/              # 主控台（设置、时长统计、脚本库、媒体处理、密码本）
    │   ├── capture-overlay/        # 截图 & 桌面墨水全屏透明画布
    │   ├── pin-image/              # Snipaste 贴图独立置顶无边框窗口
    │   ├── clipboard-popup/        # 跟随光标/底部的轻量剪贴板历史列表
    │   ├── sticky-note/            # Markdown + KaTeX 独立置顶便笺窗口
    │   └── translate-popover/      # 划词翻译悬浮小气泡
    └── shared/                     # 共享 IPC 类型定义、主题与快捷键工具
```

### 2.3 核心设计模式落地

1. **微内核与插件注册模式 (Microkernel & Plugin Registry Pattern)**
   - 主程序 (`src-tauri`) 不包含具体业务代码，仅维护 `ModuleRegistry`。10 个子功能均实现统一的 `AppModule` Trait，通过依赖注入获取数据库句柄 (`DbPool`)、事件总线 (`EventBus`) 与系统抽象层 (`OsHal`)。
2. **桥接模式 (Bridge Pattern) —— 跨平台系统抽象层 (`os-hal`)**
   - 业务层绝不直接调用 `windows::Win32` 或 `objc2`。所有与系统强相关的能力被抽象为 6 个核心 Trait：`ScreenCapturer`、`OcrProvider`、`WindowController`、`SelectionExtractor`、`BiometricAuthenticator`、`ActiveAppWatcher`，在编译期静态分发到对应平台实现（零虚函数开销）。
3. **责任链 + 策略模式 (Chain of Responsibility & Strategy Pattern)**
   - **OCR 推理降级链**：`SystemNativeOcrStrategy` (Windows.Media.Ocr / macOS Vision) $\rightarrow$ `OnnxGpuStrategy` (DirectML / CUDA / CoreML) $\rightarrow$ `OnnxNpuStrategy` (DirectML NPU / OpenVINO / CoreML ANE) $\rightarrow$ `OnnxCpuStrategy` (AVX2 / NEON)。当用户开启自定义权重或某一级硬件初始化失败时，自动沿责任链无感降级。
   - **划词捕获降级链**：`AccessibilityTextStrategy` (UIAutomation / AXUIElement 原生读取选中文本，不污染剪贴板) $\rightarrow$ `SimulatedCopyStrategy` (备份当前剪贴板 $\rightarrow$ 发送虚拟 `Ctrl+C`/`Cmd+C` $\rightarrow$ 读取选中文本 $\rightarrow$ 恢复原剪贴板)。
4. **发布-订阅模式 (Observer / Event Bus Pattern)**
   - 全局鼠标钩子 ([`rdev`](https://github.com/Narsil/rdev))、剪贴板变更监听、前台窗口切换事件统一向 `InternalEventBus` 发布领域事件（如 `SystemEvent::MouseDragSelectionEnded`、`SystemEvent::SuppressClipboardCapture`），各业务模块按需订阅，避免重复挂载系统底层 Hook。

### 2.4 极致省内存策略：WebView 生命周期管理

传统 Tauri / Electron 应用内存高的主因是在后台隐藏（`window.hide()`）了多个 WebView 实例（每个 WebView2/WKWebView 进程组占用 40~120MB）。本项目在 `WindowPoolManager` 中强制实行三级窗口生命周期策略：

- **Level 0：纯后台常驻（0 WebView 窗口）**
  - 应用开机自启后，**不创建任何前端 WebView 窗口** (`tauri.conf.json` 中 `windows: []` 为空)。仅运行 Rust 原生守护进程、系统托盘图标 (`TrayIcon`)、全局快捷键监听、剪贴板监听与使用时长计时器，常驻内存稳定在 **15~25 MB**。
- **Level 1：瞬态弹窗（Transient Window —— 用完即毁或延迟回收）**
  - 适用模块：**截图画布、桌面注释、剪贴板弹窗、划词翻译气泡、主控台（设置/统计/多媒体/脚本/密码本）**。
  - 快捷键触发时通过 `WebviewWindowBuilder::new(...)` 动态创建窗口；关闭或完成操作后调用 `window.destroy()` 彻底销毁 WebView 进程，立即归还系统内存。
- **Level 2：驻留贴顶窗口（Pinned Resident Window —— 仅在钉住期间驻留）**
  - 适用模块：**Snipaste 钉住的贴图窗口、钉在桌面的 Markdown 便笺**。
  - 仅当桌面上存在处于“已钉住”状态的贴图或便笺时保留对应窗口；一旦关闭贴图或便笺，立即销毁对应 WebView 实例。

## 3. 主程序公共基础设施设计

### 3.1 插件化模块接入标准 (`AppModule` Trait 与主程序注册表)

所有 10 个子功能必须实现以下标准接口，并在 `src-tauri/src/main.rs` 中统一挂载：

```rust
// crates/app-core/src/module.rs
use tauri::{AppHandle, Runtime};
use crate::{config::AppConfig, db::DbPool, event_bus::EventBus, shortcut::ShortcutSpec};

pub struct ModuleContext<R: Runtime> {
    pub app_handle: AppHandle<R>,
    pub db: DbPool,
    pub event_bus: EventBus,
    pub config: Arc<RwLock<AppConfig>>,
}

pub trait AppModule<R: Runtime>: Send + Sync + 'static {
    /// 模块唯一标识符 (如 "screenshot", "clipboard")
    fn id(&self) -> &'static str;
    /// 模块显示名称
    fn name(&self) -> &'static str;
    /// 数据库表结构增量迁移 SQL (纳入主程序统一迁移序列)
    fn migrations(&self) -> Vec<&'static str> { vec![] }
    /// 声明该模块的默认跨平台快捷键
    fn default_shortcuts(&self) -> Vec<ShortcutSpec> { vec![] }
    /// 主程序启动时调用：用于启动轻量后台任务（仅限剪贴板、使用时长统计等常驻模块）
    fn on_init(&self, ctx: &ModuleContext<R>) -> anyhow::Result<()>;
    /// 全局快捷键触发回调
    fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext<R>) -> anyhow::Result<()>;
    /// 主程序退出或休眠前清理资源（如还原被置顶的外部窗口、刷写计时数据、擦除密码本密钥）
    fn on_shutdown(&self, ctx: &ModuleContext<R>) -> anyhow::Result<()>;
}
```

### 3.2 跨系统快捷键归一化与全局输入钩子

基于官方插件 [`tauri-plugin-global-shortcut`](https://v2.tauri.app/plugin/global-shortcut/) 与底层输入监听库 [`rdev`](https://github.com/Narsil/rdev)，设计双层快捷键与输入架构：

1. **逻辑修饰键抽象 (`LogicalHotkey`)**：
   - 在配置文件中统一使用平台无关的逻辑表达式，由 `ShortcutNormalizer` 在运行时转换为当前系统的物理按键：
     - `PrimaryMod` $\rightarrow$ Windows 映射为 `Ctrl`，macOS 映射为 `Command (Super)`
     - `SecondaryMod` $\rightarrow$ Windows 映射为 `Alt`，macOS 映射为 `Option`
     - `MetaMod` $\rightarrow$ Windows 映射为 `Win`，macOS 映射为 `Control`
   - 例如：剪贴板呼出配置为 `"PrimaryMod+Shift+V"`，在 Windows 上自动注册 `Ctrl+Shift+V`，在 macOS 上自动注册 `Cmd+Shift+V`。
2. **快捷键冲突检测与热重载**：
   - 注册快捷键时若返回冲突错误，主程序向系统托盘推送通知并在设置面板高亮冲突项，绝不导致主程序崩溃。

### 3.3 零打包运行时探测器 (`EnvProbeService`)

为遵守“不打包任何重型运行时，依赖系统环境变量探测”的原则，主程序内置 `EnvProbeService`：

- **macOS GUI 环境变量修补 (`fix-path-env`)**：macOS 下通过 Finder/Launchpad 启动的 `.app` 默认不继承用户 `~/.zshrc` 或 `~/.bash_profile` 中的 `PATH`（会导致找不到 Homebrew 安装的 `ffmpeg`、`node`、`python3`）。主程序启动时首先通过 `$SHELL -ilc "echo -n $PATH"` 同步用户终端完整环境变量。
- **探测优先级链**：
  1. 优先读取用户在主程序设置中手动指定的二进制绝对路径（如 `custom_paths.ffmpeg`）。
  2. 遍历系统 `PATH` 查找目标可执行文件（`ffmpeg`, `ffprobe`, `node`, `python3`, `cscript`, `osascript`）。
  3. 执行 `<bin> --version` 校验可用性并缓存版本号。
  4. 若未探测到，前端对应功能区展示非侵入式的“环境未就绪引导卡片”（附带一键复制安装命令及手动指定路径入口）。

### 3.4 本地 SQLite 存储架构与跨版本平滑升级方案

1. **SQLite 存储引擎 (`app.db`)**：
   - 采用 [`rusqlite`](https://github.com/rusqlite/rusqlite)（开启 `bundled` 特性与 WAL 模式 `PRAGMA journal_mode=WAL;`）结合 [`rusqlite_migration`](https://github.com/cljoly/rusqlite_migration)。
   - **数据库自动升级流程**：每次启动检测 `PRAGMA user_version`；若低于当前代码版本，先将 `app.db` 备份为 `backups/app.db.v{old}.{timestamp}.bak`，再在事务内按序执行增量 SQL 迁移。
2. **配置文件向前/向后兼容升级 (`config.json`)**：
   - 所有配置结构体及新增字段标记 `#[serde(default)]`（缺省填充默认值），并使用 `#[serde(flatten)] pub extra: Map<String, Value>` 保留未知字段；破坏性结构变更通过 `ConfigMigrationPipeline` 按 `"config_version"` 链式转换（`v1 -> v2 -> v3`）。

## 4. 10 大子功能主程序接入点全景矩阵

| 序号 | 子功能模块 | Crate 名称 | 主程序运行模式 | 窗口 Label 与销毁策略 | 核心 OS HAL 依赖 | SQLite 数据表 | 默认跨平台快捷键 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **01** | **增强截图 (含 OCR/长截图/GIF)** | `mod-screenshot` | 按需唤醒 (贴图驻留) | `capture-overlay` (瞬态)<br>`pin-img-{id}` (置顶驻留) | `ScreenCapturer`<br>`OcrProvider`<br>`InputSimulator` | `ocr_models_meta` | `F1` (截图)<br>`Shift+F1` (长截图)<br>`SecondaryMod+F1` (GIF) |
| **02** | **剪切板管理 (类 Ditto)** | `mod-clipboard` | **后台常驻监听** | `clipboard-popup` (瞬态弹窗) | 剪贴板序列号/ChangeCount 监听 + `InputSimulator` | `clipboard_items` (LRU 100) | `PrimaryMod+Shift+V` |
| **03** | **桌面屏幕注释 (类 gInk)** | `mod-screen-ink` | 按需唤醒 | `screen-ink-canvas` + `screen-ink-toolbar` (瞬态) | `WindowController::set_ignore_cursor_events` | 无 (配置存于 `config.json`) | `PrimaryMod+Shift+D` |
| **04** | **桌面置顶便笺 (MD+公式)** | `mod-sticky-note` | 启动时按需恢复已钉住便笺 | `sticky-note-{id}` (置顶驻留，关闭即毁) | `WindowController` (置顶与毛玻璃) | `sticky_notes` | `PrimaryMod+Shift+N` |
| **05** | **应用使用时长统计 (类 TAI)** | `mod-app-usage` | **后台常驻采样** | 挂载于 `dashboard` 主控台 `/usage` 路由 (瞬态) | `ActiveAppWatcher`<br>`IdlePowerDetector` | `tracked_apps`<br>`app_usage_hourly`<br>`app_usage_sessions` | 无 (主控台查看) |
| **06** | **自定义脚本收录与执行** | `mod-script-runner` | 按需唤醒 (支持单脚本绑定快捷键) | 挂载于 `dashboard` 主控台 `/scripts` 路由 + 托盘子菜单 | `EnvProbeService` (`node`/`python`/`cscript`/`osascript`) | `custom_scripts`<br>`script_run_logs` | 用户自定义每条脚本快捷键 |
| **07** | **全局划词翻译** | `mod-translator` | **后台轻量鼠标钩子** (可一键开关) | `translate-popover` (瞬态跟随鼠标气泡) | `SelectionExtractor` (UIAutomation / AXUIElement) | `translation_providers`<br>`translation_history` | 划词悬浮图标触发 或 `PrimaryMod+Shift+T` |
| **08** | **基础多媒体处理** | `mod-media-proc` | 按需唤醒 | 挂载于 `dashboard` 主控台 `/media` 路由 (瞬态) | `EnvProbeService` (`ffmpeg`/`ffprobe`) | `media_task_history` | 无 (主控台或文件拖拽触发) |
| **09** | **窗口置顶工具 (类 DeskPins)** | `mod-window-pin` | 按需唤醒 (有置顶目标时后台守护) | `window-pin-picker` (瞬态准星拾取器) | `WindowController::set_external_window_topmost` | 无 (内存维护句柄，退出自动还原) | `PrimaryMod+Shift+P` |
| **10** | **生物识别密码本** | `mod-password-vault` | 按需唤醒 (解锁后 3 分钟自动重锁并擦除内存) | 挂载于 `dashboard` 主控台 `/vault` 路由 (瞬态) | `BiometricAuthenticator` (Windows Hello / Touch ID) | `password_vault_entries`<br>`vault_meta` | `PrimaryMod+Shift+K` |

## 5. 10 大子功能模块深度拆解与主程序接入点规范

### 5.1 子功能 01：增强截图工具 (基础截图 / 注释 / 贴图 / OCR / 长截图 / GIF 录制)

#### 1. 技术选型与实现机制
- **屏幕与窗口捕获**：采用 [`xcap`](https://github.com/nashaofu/xcap) 获取多显示器拓扑、窗口列表边界与高速帧捕获；Windows 下针对高帧率 GIF 与长截图结合 [`windows-capture`](https://github.com/NiiightmareXD/windows-capture) (Windows Graphics Capture API)，macOS 下调用 `ScreenCaptureKit` / `CGWindowListCreateImage`。
- **Snipaste 核心闭环（截图 + 注释 + 钉住 + 缩放）**：
  - 触发截图瞬间，Rust 端立即抓取全屏快照存入内存 Buffer（通过二进制 IPC `tauri::ipc::Response` 零序列化拷贝传给前端），随后弹出全屏无边框窗口 `capture-overlay`。
  - 前端使用 HTML5 Canvas 渲染快照、放大镜取色器（支持 `RGB/HEX` 复制）及矢量注释图层（矩形、箭头、马赛克、序号标尺、文字）。
  - **钉住（Pin to Screen）**：点击“钉住”时，Rust 端裁剪选区图像并创建独立无边框置顶窗口 `pin-img-{uuid}`，支持鼠标滚轮无极缩放（10%~500%）、`Shift+滚轮` 调节透明度、双击缩成迷你色块。
- **OCR 引擎（系统原生优先 + 自定义模型硬件降级）**：
  - **第一优先级（零体积系统原生 OCR）**：
    - Windows：通过 [`windows-rs`](https://github.com/microsoft/windows-rs) 调用 `Windows::Media::Ocr::OcrEngine::TryCreateFromUserProfileLanguages()`。
    - macOS：通过 [`objc2`](https://github.com/madsmtm/objc2) 调用 Apple `Vision` 框架的 `VNRecognizeTextRequest`（`VNRequestTextRecognitionLevelAccurate`）。
  - **第二优先级（用户自定义模型权重 + 硬件自适应降级）**：
    - 当用户在设置中指定自定义 `.onnx` 模型权重文件（如 PaddleOCR 的 `det.onnx` + `rec.onnx` + `keys.txt`）时，通过 [`ort` (ONNX Runtime Rust 绑定)](https://github.com/pykeio/ort) 加载模型，按 **GPU $\rightarrow$ NPU $\rightarrow$ CPU** 顺序注册 ExecutionProviders：
      ```rust
      let session = Session::builder()?
          .with_execution_providers([
              #[cfg(target_os = "windows")]
              CUDAExecutionProvider::default().build(),
              #[cfg(target_os = "windows")]
              DirectMLExecutionProvider::default().build(), // 覆盖 Windows GPU 与 DirectML NPU
              #[cfg(target_os = "windows")]
              OpenVINOExecutionProvider::default().with_device_type("NPU").build(),
              #[cfg(target_os = "macos")]
              CoreMLExecutionProvider::default().with_ane_only(false).build(), // 覆盖 Apple GPU 与 ANE (NPU)
              CPUExecutionProvider::default().build(), // CPU 兜底
          ])?
          .commit_from_file(user_model_path)?;
      ```
- **长截图功能（类 QQ NT 滚动拼接）**：
  - 用户框选区域开启“长截图”后，主程序将该选区设为鼠标穿透 (`set_ignore_cursor_events(true)`)，通过 [`enigo`](https://github.com/enigo-rs/enigo) 自动向选区中心发送滚轮事件（或由用户手动滚动）。Rust 后台线程以 15 FPS 抓取选区图像，剔除首尾固定导航栏后，计算相邻帧行特征哈希与归一化互相关（NCC）定位纵向偏移量 $\Delta y$ 实时拼接。
- **GIF 录制功能（支持自选分辨率）**：
  - 用户框选录制区域并选择输出分辨率（原画 / 1080p / 720p / 480p / 自定义宽高）与帧率（10/15/24/30 FPS）。Rust 后台线程定时捕获选区帧，使用 [`image`](https://github.com/image-rs/image) 缩放后送入 [`gifski`](https://github.com/imageoptim/gifski) 流式量化编码生成高质量 GIF。

#### 2. 在主程序中的明确接入点 (`crates/mod-screenshot`)
- **`AppModule` 挂载实现**：
  ```rust
  impl<R: tauri::Runtime> AppModule<R> for ScreenshotModule {
      fn id(&self) -> &'static str { "screenshot" }
      fn name(&self) -> &'static str { "增强截图与贴图" }
      fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
          vec![
              ShortcutSpec::new("screenshot:capture", "F1", "启动区域截图"),
              ShortcutSpec::new("screenshot:scroll", "Shift+F1", "启动滚动长截图"),
              ShortcutSpec::new("screenshot:gif", "SecondaryMod+F1", "启动 GIF 录制"),
              ShortcutSpec::new("screenshot:ocr", "PrimaryMod+Shift+O", "截图并立即 OCR"),
          ]
      }
      fn on_init(&self, _ctx: &ModuleContext<R>) -> anyhow::Result<()> { Ok(()) } // 纯按需懒加载，常驻内存为 0
      fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext<R>) -> anyhow::Result<()> {
          self.freeze_screen_and_open_overlay(ctx, action_id)
      }
  }
  ```
- **Tauri IPC 通信契约**：
  - Commands：`screenshot_get_frozen_frame`, `screenshot_pin_to_screen`, `screenshot_execute_ocr`, `screenshot_start_scroll`, `screenshot_stop_scroll`, `screenshot_start_gif`, `screenshot_stop_gif`
  - Events：`screenshot://scroll-frame-stitched`、`screenshot://gif-recording-stats`、`screenshot://ocr-provider-fallback`
- **窗口接入策略**：`capture-overlay` 完成截图后立即 `destroy()`；`pin-img-{uuid}` 仅在贴图存在时驻留，关闭贴图即 `destroy()`；OCR Session 在闲置 180 秒后自动释放。

### 5.2 子功能 02：剪切板管理器 (类 Ditto)

#### 1. 技术选型与实现机制
- **系统剪贴板监听与读写**：结合 [`clipboard-rs`](https://github.com/ChurchTao/clipboard-rs) 与 [`arboard`](https://github.com/1Password/arboard)。Windows 下注册 `AddClipboardFormatListener` 监听 `WM_CLIPBOARDUPDATE` 事件（零轮询开销）；macOS 下每 250ms 检查 `NSPasteboard::generalPasteboard().changeCount()`。
- **去重与 LRU-100 自动淘汰**：对捕获的文本或图片计算 `BLAKE3` 哈希值；已存在相同哈希时仅更新 `updated_at` 置顶；图片压缩为 WebP 落盘至 `clipboard_blobs/{hash}.webp`，超出 100 条的非固定记录自动清理。
- **快速粘贴上屏**：选中条目后隐藏弹窗、归还前序焦点窗口，写入剪贴板并通过 `enigo` 发送 `Ctrl+V` / `Cmd+V`。

#### 2. 在主程序中的明确接入点 (`crates/mod-clipboard`)
- **`AppModule` 挂载实现**：在 `on_init` 启动剪贴板监听线程，并订阅 `InternalEventBus::SuppressClipboard(bool)` 事件（配合划词翻译与密码本防污染）；在 `on_shortcut("clipboard:toggle_popup")` 唤醒跟随光标的 `clipboard-popup` 窗口。
- **Tauri IPC 通信契约**：
  - Commands：`clipboard_list_items(keyword, limit)`, `clipboard_paste_item(id, as_plain_text)`, `clipboard_toggle_pin(id)`, `clipboard_delete_item(id)`, `clipboard_clear_all()`
  - Events：`clipboard://updated`
- **SQLite 表结构 (`clipboard_items`)**：
  ```sql
  CREATE TABLE IF NOT EXISTS clipboard_items (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      item_type TEXT NOT NULL CHECK(item_type IN ('text', 'image')),
      content_text TEXT,
      blob_rel_path TEXT,
      thumb_base64 TEXT,
      content_hash TEXT NOT NULL UNIQUE,
      source_app_name TEXT,
      is_pinned INTEGER NOT NULL DEFAULT 0,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
  );
  ```

### 5.3 子功能 03：桌面屏幕注释 (类 gInk)

#### 1. 技术选型与实现机制
- **双窗口分离策略（解决全屏鼠标穿透与工具栏点击冲突）**：
  - **`screen-ink-canvas`**：全屏无边框透明画布窗口，前端采用 [`perfect-freehand`](https://github.com/steveruizok/perfect-freehand) + Canvas 2D 绘制压感墨迹、荧光笔、2 秒消隐激光笔、几何图形与橡皮擦。当切换到“操作桌面模式”时，对该窗口调用 `set_ignore_cursor_events(true)`，保留屏幕墨迹的同时允许鼠标直接操作底层应用。
  - **`screen-ink-toolbar`**：`420x56` 独立置顶胶囊工具栏窗口，始终保持可点击。
- **合成保存**：点击保存时，Rust 端调用 `xcap` 截取当前屏幕底图，并与前端传入的透明墨迹 PNG 图层合并导出。

#### 2. 在主程序中的明确接入点 (`crates/mod-screen-ink`)
- **`AppModule` 挂载实现**：绑定默认快捷键 `PrimaryMod+Shift+D`（进入/退出屏幕注释）、`PrimaryMod+Shift+E`（切换画笔/鼠标穿透），退出模式时同步 `destroy()` 画布与工具栏两个窗口，后台 0 内存占用。
- **Tauri IPC 通信契约**：
  - Commands：`ink_set_passthrough(passthrough: bool)`, `ink_save_snapshot(ink_png_bytes, copy_to_clipboard)`, `ink_exit_session()`
  - Events：`screen-ink://mode-changed`、`screen-ink://clear-canvas`

### 5.4 子功能 04：桌面置顶富文本便笺 (Markdown + LaTeX)

#### 1. 技术选型与实现机制
- **Markdown + 公式实时渲染**：前端采用插件化所见即所得编辑器 [`Milkdown`](https://github.com/Milkdown/milkdown) 集成 [`KaTeX`](https://github.com/KaTeX/KaTeX)，支持 GFM 语法、待办列表、代码块高亮及行内/块级数学公式（`$E=mc^2$`、`$$\int f(x)dx$$`）即时渲染。
- **窗口特性**：每个便笺对应独立无边框窗口 `sticky-note-{uuid}`，支持拖拽边缘调整大小 (`resizable: true`)、钉在顶部 (`always_on_top`)、透明度调节与内容 500ms 防抖写入 SQLite。

#### 2. 在主程序中的明确接入点 (`crates/mod-sticky-note`)
- **`AppModule` 挂载实现**：在 `on_init` 查询 `sticky_notes` 表中 `is_open = 1` 的记录并按原坐标恢复窗口（若无打开的便笺则不创建任何 WebView）；快捷键 `PrimaryMod+Shift+N` 新建便笺。
- **Tauri IPC 通信契约**：
  - Commands：`sticky_create_note`, `sticky_get_note(id)`, `sticky_save_content(id, content_md)`, `sticky_update_window_state(id, x, y, w, h, pinned, opacity)`, `sticky_close_note(id)`
- **SQLite 表结构 (`sticky_notes`)**：
  ```sql
  CREATE TABLE IF NOT EXISTS sticky_notes (
      id TEXT PRIMARY KEY,
      title TEXT NOT NULL DEFAULT '',
      content_md TEXT NOT NULL DEFAULT '',
      theme_color TEXT NOT NULL DEFAULT 'amber',
      opacity REAL NOT NULL DEFAULT 0.95,
      pos_x INTEGER, pos_y INTEGER,
      width INTEGER NOT NULL DEFAULT 320,
      height INTEGER NOT NULL DEFAULT 300,
      is_pinned INTEGER NOT NULL DEFAULT 1,
      is_open INTEGER NOT NULL DEFAULT 1,
      updated_at INTEGER NOT NULL
  );
  ```

### 5.5 子功能 05：应用使用时长统计 (类 Windows TAI)

#### 1. 技术选型与实现机制
- **前台驻留感知**：基于 [`active-win-pos-rs`](https://github.com/dimusic/active-win-pos-rs) 与 `os-hal::ActiveAppWatcher`（Windows `GetForegroundWindow` + `SetWinEventHook`；macOS `NSWorkspaceDidActivateApplicationNotification`）获取当前焦点应用的路径/Bundle ID、名称、窗口标题与图标。
- **息屏与离开（AFK）过滤**：通过 Windows `GetLastInputInfo` / `WM_WTSSESSION_CHANGE` 与 macOS `CGEventSourceSecondsSinceLastEventType` / `NSWorkspaceScreensDidSleepNotification` 检测锁屏、息屏与键鼠空闲（默认超过 300 秒无输入且无音频输出时暂停计时）。
- **低 I/O 批量落盘**：在 Rust 内存中按秒累加当前活跃会话，仅在焦点切换或每满 60 秒时执行一次 SQLite `UPSERT`。

#### 2. 在主程序中的明确接入点 (`crates/mod-app-usage`)
- **`AppModule` 挂载实现**：在 `on_init` 启动后台低功耗统计协程；在 `on_shutdown` 将内存未落盘的活跃会话刷入数据库。
- **Tauri IPC 通信契约**：
  - Commands：`usage_get_overview(start_date, end_date)`, `usage_get_hourly_breakdown(date)`, `usage_set_app_category(app_id, category, ignored)`
- **窗口与 SQLite 表结构**：挂载于 `dashboard` 主控台窗口的 `/usage` 路由；包含 `tracked_apps`、`app_usage_hourly (date_str, hour_of_day, app_id, active_seconds)` 与 `app_usage_sessions` 三张表。

### 5.6 子功能 06：自定义脚本收录与一键执行

#### 1. 技术选型与实现机制
- **按系统定制脚本接口（基于 `EnvProbeService` 探测系统 `PATH`）**：
  - **跨平台通用**：JavaScript (`node` / `bun` / `deno`)、Python (`python3` / `python -u`)。
  - **Windows 专属**：VBScript (`cscript.exe //NoLogo`)、PowerShell (`pwsh.exe` / `powershell.exe -ExecutionPolicy Bypass`)、Batch (`cmd.exe /c`)，均附加 `CREATE_NO_WINDOW` 避免黑框闪烁。
  - **macOS 专属**：AppleScript (`osascript`)、Shell (`/bin/zsh` / `/bin/bash`)。
- **多触发入口**：支持为单条脚本绑定全局快捷键、挂载至系统托盘“快捷脚本”子菜单，或在主控台一键运行，并自動注入 `$AIO_CLIPBOARD` 与 `$AIO_SELECTION` 环境变量。

#### 2. 在主程序中的明确接入点 (`crates/mod-script-runner`)
- **`AppModule` 挂载实现**：在 `on_init` 读取 `custom_scripts` 表，动态注册用户脚本快捷键 `script:run:{id}` 并生成托盘子菜单。
- **Tauri IPC 通信契约**：
  - Commands：`script_probe_interpreters`, `script_list`, `script_save`, `script_delete`, `script_execute(script_id, args)`, `script_kill(run_id)`
  - Events：`script://log-stream`（流式推送 `stdout`/`stderr`）、`script://execution-finished`
- **SQLite 表结构 (`custom_scripts`)**：存储 `id, name, script_type, is_inline, content_or_path, working_dir, hotkey, show_in_tray, timeout_secs, output_handling`。

### 5.7 子功能 07：全局划词翻译

#### 1. 技术选型与实现机制
- **选中文本双通道提取 (`os_hal::SelectionExtractor`)**：
  1. **原生无障碍通道（首选，零剪贴板干扰）**：Windows 通过 `UIAutomation` (`UIA_TextPatternId::GetSelection`) 读取；macOS 通过 `AXUIElementCopyAttributeValue(focused, kAXSelectedTextAttribute)` 读取。
  2. **模拟复制兜底通道**：当目标应用不支持无障碍读取时，向模块 02 发送 `SuppressClipboard(true)` $\rightarrow$ 备份剪贴板 $\rightarrow$ `enigo` 发送 `Ctrl+C`/`Cmd+C` $\rightarrow$ 读取文本 $\rightarrow$ 还原剪贴板 $\rightarrow$ 解除屏蔽。
- **多源翻译 API 网关**：默认调用 Google 翻译 API，支持用户在设置中自定义接入 DeepL、百度/腾讯/有道及任意 OpenAI 兼容格式大模型 API（支持 SSE 流式输出）。

#### 2. 在主程序中的明确接入点 (`crates/mod-translator`)
- **`AppModule` 挂载实现**：订阅 `InternalEventBus` 的鼠标拖拽选词结束事件（弹出 20x20 悬浮译字小图标，点击后展开 `translate-popover` 气泡窗）或响应 `PrimaryMod+Shift+T` 直接翻译。
- **Tauri IPC 通信契约**：
  - Commands：`translator_translate(text, from, to, provider_id)`, `translator_save_provider`, `translator_toggle_selection_hook(enabled)`
  - Events：`translator://stream-chunk`
- **SQLite 表结构 (`translation_providers`)**：存储 `id, name, provider_type, endpoint_url, api_key_cipher, extra_config_json, is_default`。

### 5.8 子功能 08：基础多媒体处理 (图片/音视频转换与裁剪)

#### 1. 技术选型与实现机制
- **图片格式互转（纯 Rust 原生内置，零外部依赖）**：基于 [`image`](https://github.com/image-rs/image) Crate + `rayon` 并行处理，开箱即用支持 `PNG/JPG/WebP/AVIF/ICO/BMP/TIFF` 批量互转、质量压缩与尺寸缩放。
- **音视频处理（基于 `EnvProbeService` 探测调用系统 `ffmpeg` / `ffprobe`）**：
  - 涵盖**音频格式互转** (`MP3/WAV/FLAC/AAC/M4A/OGG`)、**音视频时间轴裁剪** (`-ss ... -to ... -c copy` 无损极速裁剪或精准重编码)、**视频提取音频** (`-vn`)、**音量调整** (`-af "volume=..."` 或 `loudnorm`)。
  - 通过解析 `ffmpeg -progress pipe:1` 的 `out_time_us` 实时向前端推送处理进度百分比。

#### 2. 在主程序中的明确接入点 (`crates/mod-media-proc`)
- **`AppModule` 挂载实现**：纯按需唤醒，内部维护 `tokio::sync::Semaphore` 控制并发任务数；在 `on_shutdown` 中安全终止未完成的 `ffmpeg` 子进程。
- **Tauri IPC 通信契约**：
  - Commands：`media_check_ffmpeg_env`, `media_inspect_file`, `media_convert_images`, `media_submit_av_job`, `media_cancel_job`
  - Events：`media://job-progress`（推送 `{ job_id, progress, speed }`）
- **窗口接入策略**：挂载于 `dashboard` 主控台窗口的 `/media` 路由，支持系统文件拖拽 (`tauri://drag-drop`) 批量添加任务。

### 5.9 子功能 09：任意窗口置顶小工具 (类 DeskPins)

#### 1. 技术选型与实现机制
- **双触发模式**：支持按下 `PrimaryMod+Shift+P` 直接置顶当前活动窗口，或按下 `PrimaryMod+Alt+P` 唤醒全屏准星拾取层 (`window-pin-picker`) 点击任意第三方窗口将其置顶，并在目标窗口右上角附着一枚跟随移动的红色图钉角标。
- **跨平台底层实现**：
  - **Windows**：通过 `WindowFromPoint` + `GetAncestor(hwnd, GA_ROOT)` 获取顶层 `HWND`，调用 `SetWindowPos(hwnd, HWND_TOPMOST, ...)` 设为系统置顶；通过 `SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE)` 同步图钉角标位置。
  - **macOS**：通过 `AXUIElementCopyElementAtPosition` 锁定目标外部窗口引用，采用 `kAXRaiseAction` 防遮挡守护 + 可选 `ScreenCaptureKit` 实时画中画（PiP）置顶镜像窗方案。

#### 2. 在主程序中的明确接入点 (`crates/mod-window-pin`)
- **`AppModule` 挂载实现**：内存中维护 `PinnedWindows` 句柄表；**关键生命周期钩子**：在 `on_shutdown` 及 `panic` 钩子中遍历所有被置顶的外部窗口，自动恢复其原始非置顶状态 (`HWND_NOTOPMOST`)。
- **Tauri IPC 通信契约**：
  - Commands：`winpin_pick_at_cursor(x, y)`, `winpin_toggle_foreground()`, `winpin_unpin(handle)`, `winpin_list_all()`

### 5.10 子功能 10：生物识别加密密码本

#### 1. 技术选型与实现机制
- **跨平台生物识别与设备密码调用 (`os_hal::BiometricAuthenticator`)**：
  - **Windows**：通过 [`windows-rs`](https://github.com/microsoft/windows-rs) 调用 `Windows::Security::Credentials::UI::UserConsentVerifier::RequestVerificationAsync` 调起原生 **Windows Hello**（指纹、人脸或系统 PIN 码）。
  - **macOS**：通过 [`objc2`](https://github.com/madsmtm/objc2) 调用 `LocalAuthentication` 框架的 `LAContext::evaluatePolicy(LAPolicyDeviceOwnerAuthentication, ...)` 调起原生 **Touch ID 或系统登录密码弹窗**。
- **Argon2id + AES-256-GCM 字段级加密**：
  - 256-bit 主数据密钥 (`DEK`) 同时由用户主密码（经 [`argon2`](https://crates.io/crates/argon2) `Argon2id` 派生 `KEK`）和系统安全凭据库（Windows DPAPI / Credential Locker，macOS Keychain）双重封装保护。
  - 密码表中的用户名、密码、TOTP 密钥、备注均使用 [`aes-gcm`](https://crates.io/crates/aes-gcm) (`Aes256Gcm`) 独立加密后存入 SQLite。
- **内存自动擦除与防泄漏**：解锁后的 `DEK` 使用 `zeroize::Zeroizing` 包装，闲置满 180 秒或关闭窗口立即清零内存；复制密码时通知模块 02 跳过历史记录并在 30 秒后清空系统剪贴板。

#### 2. 在主程序中的明确接入点 (`crates/mod-password-vault`)
- **`AppModule` 挂载实现**：在 `on_init` 启动 180 秒空闲自动上锁守卫；在 `on_shutdown` 强制擦除内存中的 `DEK`。
- **Tauri IPC 通信契约**：
  - Commands：`vault_unlock_biometric`, `vault_unlock_master_password`, `vault_lock`, `vault_list_entries(search)`, `vault_reveal_password(id)`, `vault_copy_password(id)`, `vault_upsert_entry`, `vault_delete_entry`
  - Events：`vault://locked`
- **SQLite 表结构 (`vault_meta` & `password_vault_entries`)**：
  ```sql
  CREATE TABLE IF NOT EXISTS vault_meta (
      id INTEGER PRIMARY KEY CHECK (id = 1),
      kdf_salt BLOB NOT NULL,
      encrypted_dek BLOB NOT NULL,
      dek_nonce BLOB NOT NULL,
      biometric_enabled INTEGER NOT NULL DEFAULT 1,
      auto_lock_seconds INTEGER NOT NULL DEFAULT 180,
      updated_at INTEGER NOT NULL
  );

  CREATE TABLE IF NOT EXISTS password_vault_entries (
      id TEXT PRIMARY KEY,
      title TEXT NOT NULL,
      url_domain TEXT,
      category TEXT NOT NULL DEFAULT 'login',
      username_cipher BLOB NOT NULL,
      password_cipher BLOB NOT NULL,
      totp_cipher BLOB,
      notes_cipher BLOB,
      entry_nonce BLOB NOT NULL,
      updated_at INTEGER NOT NULL
  );
  ```

## 6. 分阶段开发路线图 (Roadmap)

### Phase 1：微内核底座与高频轻量闭环（预计 2~3 周）
1. 搭建 Cargo Workspace (`app-core`, `os-hal`, `src-tauri`, `src-ui`)，实现 `AppModule` 注册器与 `WindowPoolManager`（验证后台 0 WebView 常驻内存 < 25MB）。
2. 完成 `rusqlite_migration` 数据库自动迁移框架与 `ConfigMigrationPipeline` 配置平滑升级机制。
3. 落地 **子功能 02（剪切板管理）**、**子功能 04（置顶 Markdown+LaTeX 便笺）** 与 **子功能 09（窗口置顶 DeskPins）**。

### Phase 2：视觉画布与系统级捕获（预计 3~4 周）
1. 落地 **子功能 01（增强截图）**：先完成基础截图、注释、Snipaste 贴图缩放；再接入 Windows.Media.Ocr / macOS Vision 原生 OCR 及 `ort` 自定义模型 GPU/NPU/CPU 降级；最后完成滚动长截图与 `gifski` GIF 录制。
2. 复用截图全屏透明画布底层，快速落地 **子功能 03（桌面屏幕注释 gInk）**。

### Phase 3：后台自动化与安全隐私（预计 2~3 周）
1. 落地 **子功能 05（应用使用时长统计 TAI）**：实现前台应用监听、息屏/空闲过滤与可视化报表。
2. 落地 **子功能 07（全局划词翻译）**：实现 UIAutomation / AXUIElement + 模拟复制双通道取词与多翻译源网关。
3. 落地 **子功能 10（生物识别密码本）**：打通 Windows Hello / macOS Touch ID 与 AES-256-GCM 加密及剪贴板防泄漏联动。

### Phase 4：外部运行时联动与全平台打磨（预计 2 周）
1. 实现 `EnvProbeService`（含 macOS GUI `PATH` 自动修补）。
2. 落地 **子功能 06（自定义脚本收录与执行）**（Windows 支持 JS/Py/VBS/PS，macOS 支持 JS/Py/AppleScript/Shell）。
3. 落地 **子功能 08（基础多媒体处理）**（纯 Rust 图片互转 + 系统 FFmpeg 音视频处理）。
4. 开启 Cargo LTO (`lto = true`, `codegen-units = 1`, `strip = true`) 编译优化，完成 Windows (`.msi`/`.exe`) 与 macOS (`.dmg` Universal Binary) 打包发布。
