# HANDOFF — 功能 06 自定义脚本（OpenCode / DeepSeek V4.1 Flash 1017）

## 分支与提交

- Worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/opencode-script-runner`
- 分支：`codex/feature-script-runner`
- 功能提交：`51874d5 feat(script-runner): implement module 06 custom script library and execution`
- 修复提交：`8340ae7 fix(script-runner): align script type JSON and native hotkey rollback`
- 未合并主分支、未推送远端。HANDOFF 增量为后续独立提交。

## 实现内容

### 新增 crate `crates/mod-script-runner`

- 模块 ID：`script_runner`；SQLite 迁移 v1 创建 `custom_scripts` 与 `script_run_logs`。
- 支持脚本类型：Python、Node.js、Unix Shell、AppleScript、PowerShell、Batch、VBScript。
- 解释器探测复用 `ModuleContext.env_probe`，缺失、当前系统不支持时返回明确状态与安装提示。
- 命令构造全部使用 argv 数组；内联脚本写入 `script_runtime/<run_id>/script.<ext>` 独立文件，
  不把脚本内容拼进 shell 命令字符串。Batch 使用 `cmd.exe /d /c <file>`，仍以独立参数传文件路径。
- CRUD 字段：名称、类型、内联/文件、内容或路径、默认参数数组、工作目录、快捷键、托盘显示、
  超时、输出模式、创建/更新时间。
- 执行：真实子进程、stdout/stderr 增量事件、内存日志字节上限、执行记录尾部日志持久化、
  取消、超时、退出码、启动失败和中断状态。
- Unix 使用独立进程组并在取消/超时时 `SIGKILL` 整个组；Windows 使用
  `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` 隐藏控制台，并通过
  `taskkill /PID <pid> /T /F` 终止整棵进程树。
- `on_init` 只标记上次残留的 running 记录并注册已有快捷键，不执行脚本；
  `script_save` 只持久化，不执行脚本。仅在 `script_execute`、单脚本快捷键或托盘点击时运行。
- 停用/退出时终止活动子进程、等待监控线程收尾、注销脚本快捷键、清理临时运行目录。
- 单脚本快捷键使用 `script:run:{id}`，接入现有 `ShortcutRegistry` 生命周期。保存/删除时由
  注入的 `ScriptHotkeySync` 只迁移该动作的系统注册；失败时回滚数据库与核心注册表，并保留旧
  系统快捷键。Tauri 随后重建托盘“快捷脚本”平铺菜单。
- `AIO_CLIPBOARD` / `AIO_SELECTION` 通过显式注入的 `ScriptEnvironment` 提供：
  - 生产 Tauri 宿主使用 `arboard` 读取真实剪贴板；选区使用注入的 `SelectionExtractor`。
  - `NativeOsHal` 的选区能力仍为 Unsupported，因此 UI 显示不可用且执行时不设置该变量。
  - 无适配器时使用 `UnavailableScriptEnvironment`，不读取、不伪造。
- 日志缓存限制默认 128 KiB（可配置 4 KiB–4 MiB）；执行记录保留条数默认 100（10–1000）。

### Host / IPC

- `src-tauri/src/modules.rs` 装配同一个 `Arc<ScriptRunnerModule>` 给注册表与 IPC。
- 新增 `src-tauri/src/script_env.rs`（真实剪贴板 + 选区适配）与 `src-tauri/src/script_events.rs`
  （`script://log-stream`、`script://execution-finished`、`kernel://updated`）。
- 新增 `src-tauri/src/ipc/script.rs`：
  `script_probe_interpreters`、`script_list`、`script_get`、`script_save`、`script_delete`、
  `script_execute`、`script_kill`、`script_list_runs`、`script_get_run`、`script_get_logs`、
  `script_get_settings`、`script_update_settings`。
- 所有 `script_` IPC 经 `ModuleRegistry::with_enabled("script_runner", ...)` 进入；停用时拒绝。
- 设置写入 `config.modules.settings.script_runner`，通过 `kernel.update_config` 原子持久化。
- macOS GUI 路径与生产依赖显式装配；测试宿主仍使用明确的 Unavailable 脚本环境，不冒充生产能力。

### 前端

- 新增 `src-ui/src/components/ScriptRunnerPanel.vue`：解释器/环境能力卡片、脚本库 CRUD 表单、
  运行参数（每行一个 argv）、工作目录、快捷键、托盘、超时、输出模式、运行记录、实时日志、
  取消与删除确认。
- `Dashboard.vue` 新增“06. 脚本”标签，只在模块 enabled 时显示。
- `src-ui/src/api/ipc.ts` 增加脚本强类型。
- 三语言字典 `zh-CN` / `en-US` / `ja-JP` 增加完全一致的脚本键；现有 Logo、剪贴板、便笺、
  窗口置顶与国际化不退化。
- 未新增常驻 WebView；脚本面板随 Dashboard 标签按需挂载/卸载，轮询随组件卸载停止。

## 修改的公共文件

- 根目录：`Cargo.toml`、`Cargo.lock`、`DEVELOPMENT_CHECKLIST.md`
- `crates/os-hal/src/traits.rs`：`SelectionExtractor` 增加默认 `is_available() -> false`。
- `crates/os-hal/src/platform/services.rs`：`NativeOsHal` 明确返回 false。
- `crates/os-hal/src/mock.rs`：Mock 显式返回 true（仅测试）。
- `docs/MODULE_ARCHITECTURE.md`、`docs/testing/GUI_TEST_PLAN.md`：当前挂载数更新为 02/04/06/09。
- `src-tauri/Cargo.toml`、`src-tauri/src/{lib.rs,modules.rs,tauri_app.rs,ipc/mod.rs}`
- `src-tauri/tests/{modules.rs,unit/lib.rs}`：可用模块数 3→4，并新增脚本 IPC/设置持久化测试。
- `src-ui/src/{api/ipc.ts,components/Dashboard.vue,components/Icons.vue,locales/*.json}`
- 新增 `docs/testing/GUI_SCRIPT_RUNNER_TEST_PLAN.md`。


## 修复增量（2026-10-24，提交 `8340ae7`）

### 1. ScriptType JSON 契约统一

- `AppleScript` 序列化为 `applescript`，`PowerShell` 序列化为 `powershell`，与
  `as_str()`、SQLite 存储和 Vue `ScriptKind`/options 完全一致。
- 反序列化兼容旧值 `apple_script` / `power_shell`。
- 新增模块级 `serde_json` roundtrip 测试（全部七种类型 + 两种旧别名），以及 host
  `script_save` IPC 测试：旧别名输入保存后回包和列表均为新契约值，中文/英文/日文标签未改动。

### 2. 系统快捷键失败可见并回滚

- 新增 `ScriptHotkeySync` 注入点与 `TauriScriptHotkeySync` 实现；脚本保存/删除只迁移
  `script:run:{id}` 单个动作，不再全量 `unregister_all`，因此失败不会影响其他模块快捷键。
- 注册新快捷键时先注册再注销旧快捷键；失败时旧快捷键保留，并尽力撤销刚注册的新快捷键。
- 删除快捷键失败、注册失败或切换失败时，模块回滚 SQLite 脚本记录与 `ShortcutRegistry`，
  IPC 返回包含“系统全局快捷键更新失败”的错误，不会静默成功。
- `hotkey_active` 只在同步成功后返回 true；失败路径已回滚，不再谎报。
- 新增可注入失败同步器的模块测试（新建/更新/删除三种回滚）和 host IPC 回滚测试。

### 3. Windows 进程树终止

- Windows 取消/超时/停用新增 `taskkill /PID <pid> /T /F`，以显式进程树终止补足
  `Child::kill` 只杀解释器的问题；仍保留 `CREATE_NO_WINDOW` 隐藏控制台。
- 当前环境只有 macOS ARM64 目标，未在 Windows 实机编译或运行，不能标记为 Windows 已通过。

## 真实验证结果

最终验证均在当前 worktree 执行，未使用 shell 管道掩盖测试退出码。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `TMPDIR="$PWD/.runtime_data/test-tmp/tmp" cargo test --workspace --offline --all-targets` | 退出码 0；修复后 `mod-script-runner` 20 passed；host `tests/modules` 10 passed；workspace 其余全部通过；1 个原生窗口枚举测试按设计 ignored |
| `pnpm --dir src-ui test` | 退出码 0；2 passed，0 failed（三语键结构一致、Logo 矢量结构） |
| `pnpm --dir src-ui run build` | 退出码 0；`vue-tsc --noEmit` 通过，Vite 生产构建成功 |

`mod-script-runner` 的 20 个测试覆盖修复后的 JSON 契约与快捷键同步失败回滚，并覆盖原有行为：

- 七种脚本类型的 argv 构造，包含 PowerShell/Batch/VBScript 的独立参数与路径空格；
- 内联/文件脚本 CRUD、重启持久化、初始化与保存不执行；
- 真实 Shell stdout/stderr/退出码；
- 真实 Python、Node、AppleScript 执行（解释器不可用时显式跳过并记录原因）；
- 带空格/引号参数、文件路径空格、工作目录、原文件不被改写；
- 睡眠脚本超时终止、取消、停用清理；
- 增量日志序号、日志缓存上限、final 模式抑制实时条目；
- `AIO_CLIPBOARD`/`AIO_SELECTION` 注入与 Unavailable 时不设置；
- 单脚本快捷键冲突、启停注册/注销、删除后保留历史记录；
- `applescript`/`powershell` JSON roundtrip 与旧别名读取；
- 原生快捷键同步失败时新建、更新、删除的回滚与错误可见性。

host `tests/modules` 的 10 个测试新增覆盖脚本 IPC 类型契约、设置持久化和注入失败的系统快捷键回滚。

测试临时目录通过 `TMPDIR` 指向 worktree `.runtime_data/test-tmp`，未写入用户日常数据库或文件。

## GUI 用例状态

功能专项文档：`docs/testing/GUI_SCRIPT_RUNNER_TEST_PLAN.md`。

**全部 18 条 GUI 用例标记为“未执行”。** 当前并行会话未启动 release App，也未通过 Computer Use
做实际点击、输入、托盘点击或真实系统快捷键操作。Rust/IPC/前端构建结果没有写成 GUI 通过。
需在集成后由具备桌面会话的执行者按文档补测，尤其是：

- GS-04/05/06：保存不执行、点击后真实日志与退出码；
- GS-11：运行中关闭模块的界面反馈与进程清理；
- GS-12/13：快捷键冲突提示及真实键盘触发；
- GS-14：托盘快捷脚本菜单重建；
- GS-15/16：重启持久化与删除后历史保留。

## 已知缺口与集成注意事项

1. **macOS 选区提取仍是 Unsupported**：`NativeOsHal::SelectionExtractor` 未实现，`AIO_SELECTION`
   界面显示不可用且执行时不注入；这是真实能力状态，不是本次功能回归。
2. **Windows 未实机验证**：仅 macOS ARM64 目标可用；控制台隐藏、argv 构造和
   `taskkill /T /F` 进程树终止未在 Windows 实机编译/运行，不能标记为 Windows 通过。
3. **系统快捷键极端回滚失败仍可能不一致**：适配器在新快捷键注册失败或旧键注销失败时会保留/
   恢复旧键，并尽力撤销新键；若操作系统连撤销新键也失败，会返回复合错误，此时 OS 状态需人工检查。
4. **托盘快捷键菜单为平铺项，不是嵌套 Submenu**：行为与 `show_in_tray` 一致，菜单项点击即执行；
   未使用子菜单层级。
5. **`output_handling` 的 `final` 与 `discard` 目前都抑制实时事件**，`final` 保留最终尾部日志；
   `discard` 也不清空最终尾部（界面文案为“不推送实时日志”）。若需彻底丢弃输出，需再调整。
6. **GUI 仍未执行**：本次修复未启动 release App 做真实点击/键盘/托盘回归，不能据此提交发布门槛通过结论。
7. 公共文件存在并行成员改动风险，冲突由 Codex 集成处理；本 worktree 未修改其他成员目录。
