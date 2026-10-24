# 功能 03：桌面屏幕注释（Screen Ink）交接报告 (HANDOFF.md)

## 1. 分支与提交基本信息

- **认领智能体**：Antigravity (Gemini 3.8 Flash)
- **负责功能**：子功能 03：桌面屏幕注释 (Screen Ink)
- **工作目录**：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/antigravity-screen-ink`
- **开发分支**：`codex/feature-screen-ink`
- **基线 Commit**：`68fecea`
- **功能提交 Commit**：`a15f4a6`
- **Git 状态**：已完成本地 commit，未执行 merge/push，符合不自行 merge/push 规范。

---

## 2. 详细实现内容与缺口说明

### 2.1 核心实现清单

1. **Rust 领域核心 Crate (`crates/mod-screen-ink`)**：
   - 严格遵循微内核与 OS HAL 分层规范，仅依赖 `app-core` 与 `os-hal` Traits（通过注入 `Arc<dyn ScreenCapturer>` 获取系统截屏能力，严禁直接依赖平台专有 API 或平台条件宏）；
   - 实现 `AppModule` 契约规范：模块 ID 设为 `"screen_ink"`，注册默认系统全局快捷键 `PrimaryMod+Shift+D`（进入/退出会话）、`PrimaryMod+Shift+E`（切换桌面穿透/绘图模式）；
   - 遵循零后台常驻内存与整洁生命周期目标（冷启动 0 窗口，会话启动创建双窗口，会话退出/模块关闭彻底销毁窗口并释放 WKWebView/WebView2 内存）；
   - 实现单帧真实底图捕获与像素级 Alpha 叠加合成引擎（`composite.rs`），支持 HiDPI / Retina 动态缩放对齐，生成 PNG 并通过 `arboard` 真实写入操作系统剪贴板，支持指定路径落盘。

2. **双窗口真正分离架构**：
   - **`screen-ink-canvas`**：全屏无边框透明置顶画布（`always_on_top: true`, `transparent: true`, `decorations: false`），支持 PointerEvent 压感拟合、荧光笔半透明渲染、2 秒渐变消退激光笔、直线/矩形框/方向箭头绘制及橡皮擦，支持 Undo/Redo 历史栈；
   - **`screen-ink-toolbar`**：独立浮动胶囊工具栏窗口（440x56px），具有 macOS 毛玻璃磨砂质感与拖拽手柄，置顶且**绝不穿透点击**；
   - **操作桌面模式**：通过 IPC `ink_set_passthrough`，仅对全屏画布启用 `set_mouse_passthrough(true)`，底层应用可正常点击交互，墨迹保持置顶显示；工具栏保持可点击，随时可一键切回绘图模式；
   - **双窗口实时通信**：采用标准 `BroadcastChannel('screen-ink-channel')` 结合 Tauri 事件总线，保证双窗口毫秒级零延迟状态同步。

3. **微内核装配与 IPC 通信契约**：
   - 在 `src-tauri/src/modules.rs` 与 `DesktopHost` 挂载 `ScreenInkModule`；
   - 在 `src-tauri/src/ipc/` 增加 `screen_ink.rs` 适配器，统一采用 `ink_` 前缀路由（`ink_get_status`、`ink_start_session`、`ink_exit_session`、`ink_set_passthrough`、`ink_clear_canvas`、`ink_save_snapshot`、`ink_get_config`）；
   - 所有调用均经由 `kernel.registry().with_enabled("screen_ink", ...)` 守卫，模块停用时立即拦截新请求；
   - 在 `src-tauri/src/tauri_app.rs` 中同步扩展系统托盘菜单多语言项与快捷键注册，并在模块禁用时执行针对注释双窗口的兜底销毁。

4. **主控台功能入口与多语言国际化**：
   - 在 `src-ui/src/components/Dashboard.vue` 增设「03. 屏幕注释」专属选项卡，提供直观的运行状态指示、操作桌面模式切换、清屏与启动/退出控制；
   - 在 `src-ui/src/components/Icons.vue` 补充高保真 SF Symbols 风格的画笔、荧光笔、激光笔、矩形、箭头、橡皮擦、指针与相机等矢量图标；
   - 补全简体中文 (`zh-CN.json`)、英语 (`en-US.json`)、日语 (`ja-JP.json`) 三语言字典，键名完全同构且通过严格自动化检测。

### 2.2 缺口与设计边界说明

1. **多显示器边界**：当前版本优先对齐当前主屏幕（Primary Monitor）进行全屏画布铺设与单帧捕获，HiDPI 逻辑/物理坐标已正确对齐。跨多屏任意拼接触达将在后续多屏统一拓扑方案中升级。
2. **零大型外部依赖**：未引入大型 Node 外部包，采用纯原生 TypeScript + Canvas 2D 贝塞尔曲线与压感插值实现，确保完全离线编译与极速打包。

---

## 3. 测试命令及真实结果

所有测试均在本地独立 worktree（`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/antigravity-screen-ink`）严格复用根目录缓存与 target 完成离线回归：

| 测试命令 | 测试目标 | 真实执行结果 |
| :--- | :--- | :--- |
| `CARGO_TARGET_DIR=... cargo test --workspace --offline --all-targets` | 全工作区 Rust 单元测试、生命周期测试与 Host IPC 集成测试 | **全部通过** (17 passed; 0 failed; 1 ignored[需桌面环境的窗口枚举测试]) |
| `cargo fmt --all -- --check` | Rust 代码格式规范校验 | **通过** (Exit code 0, 无格式差异) |
| `pnpm --dir src-ui run build` (`vue-tsc --noEmit && vite build`) | 前端 TypeScript 类型检查与 Vite 生产打包 | **通过** (57 modules transformed, built in 681ms) |
| `pnpm --dir src-ui test` | 前端多语言同构、SVG Logo 几何完整性、Ink 历史栈与激光笔消隐算法测试 | **全部通过** (4 passed; 0 failed) |

---

## 4. GUI 测试用例状态

编制功能专属文档：[`docs/testing/GUI_TEST_SCREEN_INK.md`](docs/testing/GUI_TEST_SCREEN_INK.md)。

- **用例总数**：14 项（涵盖积木开关、双窗口启动、压感画笔、荧光笔、2秒消隐激光笔、几何形状、橡皮擦、撤销/重做、桌面穿透交互、单帧合成保存、会话销毁、模块关闭防护、三语言切换）；
- **当前执行状态**：全部标为 **「未执行（无物理桌面会话 / 待集成阶段手工回归）」**。严格遵守任务书规范，不将 IPC 与算法单测冒充为 GUI 通过。

---

## 5. 修改公共文件列表 (用于 Codex 最终集成比对)

以下为在独立 worktree 中修改的公共基础设施文件列表，均仅增加模块 03 接入所需最小代码，未进行任何无关重构：

1. `Cargo.toml`：在 workspace `members` 列表中加入 `"crates/mod-screen-ink"`。
2. `Cargo.lock`：新增 `mod-screen-ink` 依赖关系锁定。
3. `src-tauri/Cargo.toml`：在 dependencies 中增加 `mod-screen-ink = { path = "../crates/mod-screen-ink" }`。
4. `src-tauri/src/modules.rs`：装配点注册 `ScreenInkModule` 并返回元组。
5. `src-tauri/src/lib.rs`：在 `DesktopHost` 结构体持有 `screen_ink` 句柄并对外暴露。
6. `src-tauri/src/ipc/mod.rs`：路由表挂载 `ink_` 前缀并受 `screen_ink` 模块开关保护。
7. `src-tauri/src/tauri_app.rs`：为 `screen-ink-canvas` 和 `screen-ink-toolbar` 提供主屏自适应坐标计算、系统托盘菜单多语言项与模块关闭时窗口销毁。
8. `src-tauri/tests/modules.rs`：更新挂载模块数量断言（3 -> 4），并新增模块停用时拦截 IPC 与窗口自动销毁集成测试。
9. `src-tauri/tests/unit/lib.rs`：更新模块列表数量断言（3 -> 4）。
10. `src-ui/package.json`：更新 `test` 脚本以同时运行 `i18n.test.mjs` 与 `ink.test.mjs`。
11. `src-ui/src/App.vue`：添加 `screen-ink-canvas` 与 `screen-ink-toolbar` 窗口路由挂载。
12. `src-ui/src/components/Dashboard.vue`：添加「03. 屏幕注释」选项卡、状态卡片、快捷键指南及交互控制。
13. `src-ui/src/components/Icons.vue`：补充屏幕注释专属画笔、荧光笔、激光笔、几何图形等矢量图标。
14. `src-ui/src/locales/zh-CN.json`：新增 03 模块全部中文翻译。
15. `src-ui/src/locales/en-US.json`：新增 03 模块全部英文翻译。
16. `src-ui/src/locales/ja-JP.json`：新增 03 模块全部日文翻译。

---

## 6. 集成注意事项

1. **`DesktopHost` 构造变动**：`src-tauri/src/modules.rs` 的 `mount` 函数现在返回包含 4 个模块的元组 `(builder, clipboard, sticky, pin, ink)`。其他并行分支若有新增模块，需在元组中进行合并。
2. **WebView 参数一致性**：`screen-ink-canvas` 窗口务必配置 `transparent: true` 与 `decorations: false`，在 macOS 下支持全屏透明置顶。
3. **依赖锁定**：未增加任何外部网络依赖，完全兼容已有 offline Cargo 与 pnpm 缓存。
