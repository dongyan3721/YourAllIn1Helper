# 功能 05：应用使用时长统计交接报告

## 1. 分支、工作区与提交

- 认领智能体：OpenCode
- 实际模型：`deepseek/deepseek-flash`（DeepSeek V4.1 Flash 1017）
- 专用 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/opencode-app-usage`
- 分支：`codex/feature-app-usage`
- 基线：`4689c49`
- 本地提交（未 merge / 未 push）：
  - `c51152f` feat(app-usage): add real foreground sampling and usage reports
  - `5a39f7a` feat(app-usage): add usage dashboard with zh/en/ja copy
  - 本交接/用例文档提交：见本文件所在提交 `docs(app-usage): hand off M1+M2 status and GUI cases`
- 范围：只改本 worktree；未修改主目录、其他 agent 工作区、截图/翻译/密码本目录；未改全局权限、代理或模型。

## 2. 已完成能力（M1 + M2）

### M1 真实采样与计时

- 独立 `crates/mod-app-usage`，通过 `AppUsageModule::new(Arc<dyn ActiveAppWatcher>)` 注入真实能力。
- macOS 平台实现独立文件 `crates/os-hal/src/platform/usage/macos.rs`：
  - 前台应用：`NSWorkspace.frontmostApplication`，稳定标识优先 `bundleIdentifier`；无 Bundle ID 时退回可执行文件绝对路径，最后才用 `pid:<pid>`，不再把进程名拼成 ID。
  - 窗口标题：`CGWindowListCopyWindowInfo` 读取前台应用普通窗口名；无屏幕录制权限时为空，不伪造。
  - 键鼠空闲：`CGEventSourceSecondsSinceLastEventType(CGEventSource::seconds_since_last_event_type)` 的真实秒数。
  - 锁屏/息屏：`CGSSessionCopyCurrentDictionary` 的 `CGSSessionScreenIsLocked` + `CGDisplayIsAsleep(CGMainDisplayID())`。
- 计时：`UsageClock` 可注入；生产单调时钟基于 `Instant`，墙钟用 UTC。
- 切段：按 UTC 小时边界切分，再映射为本地日期/小时；跨小时、跨日、夏令时重复小时均落入正确桶。
- 异常：采样间隔 > 5 秒（休眠/唤醒）或墙钟与单调时钟偏差 > 2 秒时，丢弃该间隔并关闭会话；不补记离线时间。
- 刷写：焦点切换、非活跃切换、每满 `flush_interval_secs`、停用/退出均刷写；写库失败会把整秒增量放回内存待重试。
- 重启：从第一个采样点重新开始，不恢复未刷写增量，不重复计时。
- 会话：`app_usage_sessions` 在会话关闭时落盘；不足 1 秒的切换不记录。

### M2 报表与设置

- `/usage` 仪表盘：日期范围（最近 7/30 天或自定义）、每日总览、小时分布、分类占比、应用分类与忽略名单、采样设置。
- 应用分类/忽略名单持久化在 `tracked_apps`；忽略后立即停止新增，历史保留，可恢复。
- 窗口标题是否保存由 `keep_window_titles` 控制；不采集按键内容、不截屏。
- 三语言：zh-CN / en-US / ja-JP 字典完整同构。
- 模块总开关沿用内核语义：关闭即停止采样并刷写，历史保留，`usage_*` IPC 被拒绝；重启保持开关。

## 3. 接口、迁移与配置

### IPC（统一经 `kernel.registry().with_enabled("app_usage", ...)`）

| 命令 | 参数 | 返回 |
| --- | --- | --- |
| `usage_get_status` | — | 采样状态、能力探测、当前配置、限制说明 |
| `usage_get_overview` | `start_date?`, `end_date?`（`YYYY-MM-DD`，默认最近 7 天，最多 366 天） | 总秒数、每日、应用排行、分类汇总 |
| `usage_get_hourly` | `date?`（默认本地今天） | 0–23 小时桶与应用明细 |
| `usage_get_apps` | — | 已跟踪应用、分类、忽略、累计秒数 |
| `usage_set_app_category` | `app_id`, `category`, `ignored` | 更新后的应用记录 |
| `usage_get_config` | — | 模块配置 |
| `usage_update_config` | `idle_threshold_secs?`, `flush_interval_secs?`, `keep_window_titles?` | 规范化后的配置 |

无新增事件。

### SQLite 迁移（`app_usage` v1）

- `tracked_apps(app_id PK, app_name, executable_path, category, ignored, window_title, first_seen_at, last_seen_at)`
- `app_usage_hourly(date_str, hour_of_day, app_id, active_seconds, PK(date_str,hour_of_day,app_id))`
- `app_usage_sessions(id, app_id, app_name, window_title, started_at, ended_at, active_seconds)`
- 只新增本模块表，不改旧六模块表或数据；所有已挂载模块迁移仍在启动时执行。

### 配置

- 新键：`modules.settings.app_usage`（`AppUsageConfig`）。
- 字段：`idle_threshold_secs`（默认 300，范围 30–86400）、`flush_interval_secs`（默认 60，范围 5–3600）、`keep_window_titles`（默认 true）。
- 旧 `usage_tracking` typed 配置保持原样，未被覆盖或删除。

## 4. 公共文件清单（供集成比对）

工作区与宿主：

- `Cargo.toml`、`Cargo.lock`：加入 `crates/mod-app-usage` workspace member 与 host 依赖。
- `src-tauri/Cargo.toml`：加入 `mod-app-usage`。
- `src-tauri/src/lib.rs`：`DesktopHost` 持有并装配 `AppUsageModule`。
- `src-tauri/src/modules.rs`：`Mounted.app_usage`，注册顺序位于现有六模块之后。
- `src-tauri/src/ipc/mod.rs`：`usage_` 前缀路由到 `app_usage`，沿用 `with_enabled`。
- `src-tauri/src/ipc/usage.rs`（新）：报表/配置 IPC 适配。

OS HAL：

- `crates/os-hal/Cargo.toml`：补充 Windows 目标 features（本机未编译验证）。
- `crates/os-hal/src/platform/mod.rs`：挂载 `usage` 子模块。
- `crates/os-hal/src/platform/services.rs`：移除旧的伪 `app.<进程名>` / Unsupported 实现。
- `crates/os-hal/src/platform/usage/{mod,macos,windows}.rs`（新）：真实能力与平台隔离。
- `crates/os-hal/tests/capabilities.rs`：增加 macOS 真实能力探测回归。

模块与测试：

- `crates/mod-app-usage/{Cargo.toml,src/{lib,config,store,tracker,report}.rs,tests/unit/lib.rs}`（新）。
- `src-tauri/tests/{unit/lib.rs,modules.rs,feature_integration.rs,support/ui_smoke.rs}`：可用模块数 6→7，新增 app_usage IPC/配置持久化测试。

前端：

- `src-ui/src/components/AppUsagePanel.vue`（新）。
- `src-ui/src/components/Dashboard.vue`：新增 `usage` 标签与面板，未改工具栏层级/事件权限修复。
- `src-ui/src/components/Icons.vue`：新增 `chart` 图标。
- `src-ui/src/api/ipc.ts`：新增 usage 类型。
- `src-ui/src/locales/{zh-CN,en-US,ja-JP}.json`：新增 `tabs.usage` 与 `appUsage` 字典。

未修改 `crates/app-core` 任何文件。

## 5. 实际执行与结果

所有命令均在本 worktree 执行，Rust 使用独立目标目录 `CARGO_TARGET_DIR=$PWD/.runtime_data/target-app-usage`，离线依赖可用。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过，无差异 |
| `cargo test -p mod-app-usage --offline` | 15 passed；覆盖空闲/锁屏、睡眠跳变、跨小时、忽略、停用刷写、重启、标题开关、能力限制 |
| `cargo test -p os-hal --offline --test capabilities` | 3 passed；macOS 真实前台/空闲/锁屏查询返回成功（未实际锁屏） |
| `cargo test --workspace --offline --all-targets` | 93 passed, 0 failed, 1 ignored（原生 Windows 窗口枚举，需登录桌面会话） |
| `pnpm --dir src-ui run build` | 通过（`vue-tsc --noEmit` + Vite；仅有既有 >500 kB chunk 提示） |
| `pnpm --dir src-ui run test` | 4 passed（含三语言字典同构） |

依赖：`pnpm install --prefer-offline` 从本机 pnpm store 复用，未下载新包；Rust 使用 `--offline`。

## 6. GUI 用例与未执行项

GUI 用例见 [`docs/testing/GUI_TEST_APP_USAGE.md`](../testing/GUI_TEST_APP_USAGE.md)，共 15 项，**全部标为未执行**。本轮没有人工桌面会话，未实际点击仪表盘、切换前台应用、锁屏、息屏或睡眠；IPC/单元测试不能替代这些证据。

明确限制：

- **音频 AFK 例外未实现**：当前没有真实系统音频活动能力。页面和 `usage_get_status.capabilities` 明确显示 `audio_afk_exception = false` 及原因，不宣称完整满足原说明书。
- **Windows 分支未验证**：已写 `platform/usage/windows.rs`（前台路径、`GetLastInputInfo`、输入桌面锁屏判断），但本 worktree 为 macOS，未编译、未实机验证；集成到 Windows 时需单独回归，必要时修正 windows-rs API。
- **真实锁屏/息屏/睡眠未实测**：只有注入时钟的单测与原生 API 返回成功；需在 GUI 阶段按用例 4–7 复核。
- **窗口标题权限**：无屏幕录制权限时标题为空；不通过其他私有方式绕过。
- **采样粒度**：固定 1 秒采样、5 秒跳变阈值、2 秒时钟漂移容差，本轮未做用户可配置。
- **会话表暂无 UI 列表**：数据已落盘，仪表盘当前展示聚合报表，不展示逐会话明细。

## 7. 集成注意

- 只做本地提交，未 merge / push。
- 合并时以 `src-tauri/src/modules.rs` 的命名装配、`ipc/mod.rs` 的单行路由和 `Dashboard.vue` 的 tabs 数组为重点，避免与其他并行功能冲突。
- 模块可用数由目录实际挂载生成，现为 7 个可用槽位；`feature_integration` 与 `ui_smoke` 的断言已同步更新。
