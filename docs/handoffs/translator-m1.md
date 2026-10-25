# 功能 07：全局划词翻译（Translator）M1 交接报告 (HANDOFF.md)

## 1. 分支、模型与提交基本信息

- **认领智能体**：Antigravity
- **模型配置**：Gemini 3.8 Flash (High)
- **工作目录**：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/antigravity-translator`
- **开发分支**：`codex/feature-translator`
- **基线 Commit**：`4689c49fc162eb4039b2a900d38d6f904c199a8d` (`4689c49`)
- **Git 状态**：所有命令与改动严格限定在本专用 worktree，未修改主工作区或其他成员目录，未执行 merge/push。
- **本地提交历史清单**：
  1. `d45b733` feat(os-hal): 实现原生选区提取与模拟复制降级契约
  2. `ab1e48f` feat(mod-translator): 实现划词翻译领域核心与网关适配
  3. `9ebb5eb` feat(host): 装配划词翻译模块、IPC路由与事件桥接
  4. `1b2b791` feat(ui): 落地划词翻译气泡、主面板与三语言适配
  5. `42d2179` docs(testing): 编制划词翻译GUI验收用例并统一代码格式
  6. `ace2ee7` docs(handoff): 补充全局划词翻译M1交付交接文档

---

## 2. M1 独立闭环交付范围与实现细节

### 2.1 原生无障碍选区提取与焦点保留 (`os-hal`)
- **保持前台焦点**：在读取选区前，严禁先弹出气泡抢占目标第三方应用（如 Safari / VS Code / 终端）焦点；优先通过 `NSWorkspace` 获取前台应用 PID 与名称。
- **macOS AXUIElement 适配**：严格调用 `AXIsProcessTrusted()` 校验权限。若未授权，准确返回 `HalError::PermissionDenied` 与中文指引；若目标控件无选区返回 `Ok(None)`；不支持时返回 `HalError::Unsupported`，严禁伪造文本。
- **模拟复制降级契约**：遵照指令，EventBus 现有单 `AtomicBool` 存在并发安全隐患，由 Codex 统一重构为具名所有者与 RAII 租约。在此之前，模拟复制降级保持显式禁用（`HalError::Unsupported("simulated_copy_fallback_disabled_pending_clipboard_lease")`），不假装实现安全模拟复制，确保不污染系统剪贴板。

### 2.2 凭据安全与隐私隔离 (`SessionCredentialStore`)
- **严禁明文落盘**：定义 `CredentialStorage` 抽象接口，实现会话级安全内存存储 `SessionCredentialStore`。API Key 仅在内存中留存，严禁序列化写入 `config.json`、SQLite 数据库或运行时控制台日志。
- **缺失凭据真实报告**：未配置 API Key 时如实向用户提示「未配置 API Key」，严禁在本地搜索用户现有私有密钥，严禁在无凭据或未授权时将选区文本外发至公网。

### 2.3 官方服务商适配与 SSE / UTF-8 跨块安全收集
- **官方提供商网关**：
  - `GoogleProvider`：适配官方 Google Cloud Translation API v2 REST 端点；
  - `DeepLProvider`：适配官方 DeepL 免费端点（`:fx`）与专业端点；
  - `OpenAIProvider`：适配标准 OpenAI 格式及本地兼容大模型端点（支持自定义 model、temperature、endpoint）。
- **UTF-8 边界安全**：实现 `Utf8ChunkBuffer`，当中日文多字节（3 字节）或 Emoji（4 字节）被 TCP / SSE 流式切断分片时，缓存未完结字节，与下一分片拼合后再转换为 Rust 有效字符串，杜绝 panic 与乱码。
- **请求防迟到与协作取消**：引入原子递增 `request_id`，发起新翻译或用户取消时立即同步 abort 上一个任务；事件管道严格校验请求序列号，迟到分片一律丢弃，绝不覆盖新结果。

### 2.4 主程序装配、最小事件权限与生命周期
- **目录接入**：在 `AppKernel` 正式挂载 `TranslatorModule`（ID `"translator"`，可用槽位增至 7 个）。
- **门禁保护**：所有 `translator_*` IPC 路由均受 `kernel.registry().with_enabled("translator", ...)` 统一管控，禁用时直接拒绝调用。
- **权限最小化**：新增 `src-tauri/capabilities/translator-events.json`，仅向 `["dashboard", "translate-popover"]` 授予 `core:event:allow-listen` 与 `core:event:allow-unlisten` 权限，不扩大任何系统核心权限。
- **生命周期**：`on_shutdown` 干净同步取消正在执行的翻译协程，并销毁 `translate-popover` 窗口，杜绝孤儿窗口与后台泄漏。

### 2.5 前端双视图与多语言
- **独立气泡视图 (`TranslatePopover.vue`)**：支持置顶切换、多语言即时对调、流式打字效果、复制译文、快捷键（Esc 关闭，Enter 重翻）与权限/凭据引导。
- **主控制台面板 (`TranslatorPanel.vue`)**：提供翻译沙箱、服务商凭据掩码输入与自定义端点维护。
- **三语言同构**：全量补齐 `zh-CN.json`、`en-US.json`、`ja-JP.json` 的 `tabs.translator` 与 `translator.*` 键值。

---

## 3. 真实测试结果清单

所有测试均在独立 worktree（`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/antigravity-translator`）内真实执行：

| 测试命令 | 测试范围与目标 | 执行结果 |
| :--- | :--- | :--- |
| `cargo test -p os-hal --offline` | 原生选区提取能力、Mock HAL、权限拒绝与不支持边界 | **全部通过** (2 passed; 0 failed; 1 ignored[需物理会话]) |
| `cargo test -p mod-translator --offline` | 划词翻译专属测试（7 项单元测试 + 4 项本地 HTTP Fixture + 1 项生命周期集成） | **全部通过** (12 passed; 0 failed) |
| `cargo test -p all-in-one-host --offline` | DesktopHost 装配、IPC 增删改查、选区提取、事件桥接、模块禁用拦截 | **全部通过** (19 passed; 0 failed) |
| `cargo test --workspace --offline --all-targets` | 全工作区（10 个 crate 及宿主）离线回归测试 | **全部通过** (全绿通过，无回退) |
| `cargo fmt --all -- --check` | Rust 代码格式规范校验 | **通过** (Exit code 0, 格式对齐) |
| `pnpm --dir src-ui run build` | 前端 TypeScript 类型检查 (`vue-tsc --noEmit`) 与 Vite 生产打包 | **通过** (69 modules transformed, 794ms) |
| `pnpm --dir src-ui test` | 前端多语言同构、SVG 图标完整性与算法回归测试 | **全部通过** (4 passed; 0 failed) |

---

## 4. 显式本地 HTTP Fixture 协议测试说明

根据任务要求，绝不使用虚假外部实测，编写显式本地 TCP/HTTP 服务器验证真实网络调用协议：
1. **Google Translate 协议测试**：校验 `POST /v2/translate`、`X-goog-api-key` 请求头与 JSON body，正确解析 `data.translations[0].translatedText`；
2. **DeepL 协议测试**：校验 `POST /v2/translate`、`Authorization: DeepL-Auth-Key` 头与 `target_lang` 大写映射；
3. **OpenAI SSE 流式分片测试**：本地服务器流式推送 `data: {...}` 分片与 `data: [DONE]`，客户端通过 `mpsc` 完整接收追加并拼接为最终译文；
4. **并发与取消丢弃测试**：服务端模拟 300ms 网络延迟，客户端发起后立即 `cancel_current()`，断言延迟到达后的分片被自动丢弃，sink 收到事件数为 0。

---

## 5. GUI 测试用例与状态

编制专属用例文档：[`docs/testing/TRANSLATOR_GUI_TEST_PLAN.md`](../testing/TRANSLATOR_GUI_TEST_PLAN.md)。
- **用例总数**：13 项（涵盖全局快捷键唤出、第三方前台焦点保留、无选区输入、权限引导、流式显示、Esc 关闭、主面板凭据配置与启停拦截）。
- **当前状态**：全部明确标为 **「未执行（无物理桌面会话 / 待集成阶段手工回归）」**。不把单元/集成测试冒充为物理 GUI 通过。

---

## 6. 修改的公共文件清单 (供 Codex 集成核对)

1. `Cargo.toml`：将 `crates/mod-translator` 加入 workspace members；
2. `Cargo.lock`：更新依赖图谱；
3. `crates/os-hal/src/platform/mod.rs` & `services.rs`：挂载 `selection` 模块并移除旧未实现桩；
4. `crates/os-hal/tests/capabilities.rs` & `tests/unit/platform.rs`：更新能力边界断言并新增 `SelectionExtractor` 接口测试；
5. `src-tauri/Cargo.toml`：添加 `mod-translator` 依赖；
6. `src-tauri/src/modules.rs`：在 `Mounted` 结构体装配 `TranslatorModule`；
7. `src-tauri/src/lib.rs`：在 `DesktopHost` 持有 `translator` 模块实例；
8. `src-tauri/src/ipc/mod.rs`：添加 `translator_` 路由分发；
9. `src-tauri/src/tauri_app.rs`：注册 `ACTION_TRANSLATOR` 全局动作、托盘菜单项与 `translator://stream-chunk` 原生事件桥接；
10. `src-tauri/tests/feature_integration.rs` & `tests/modules.rs` & `tests/unit/lib.rs`：将可用模块数断言从 6 调整为 7，新增划词翻译 IPC 全流程测试；
11. `src-ui/src/App.vue`：挂载 `translate-popover` 独立窗口模式；
12. `src-ui/src/components/Dashboard.vue`：在 `tabs` 中追加划词翻译选项卡；
13. `src-ui/src/components/Icons.vue`：新增 `translate` 矢量图标；
14. `src-ui/src/locales/zh-CN.json`、`en-US.json`、`ja-JP.json`：全量同步多语言字典。

---

## 7. 共享能力与后续协作待办项 (M2 展望)

1. **共享剪贴板租约 (由 Codex 统一主导)**：待 Codex 完成带所有者标识与 RAII 作用域的租约后，在 M2 接入模拟复制兜底通道（备份剪贴板 $\rightarrow$ `Cmd/Ctrl+C` $\rightarrow$ 提取选区 $\rightarrow$ 原样还原剪贴板）；
2. **全局鼠标拖拽取词钩子 (M2)**：监听鼠标抬起与移动停顿事件，展示 20x20 悬浮译字图标；
3. **国内翻译服务商扩展 (M2)**：按官方 API 规范分批扩展百度/腾讯/有道翻译，需在用户配置可用密钥时启用；
4. **系统级原生凭据库打通**：将内存会话存储升级为 macOS Keychain / Windows Credential Locker 安全封装。
