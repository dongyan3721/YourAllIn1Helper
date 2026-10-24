# HANDOFF — 功能 06 自定义脚本（OpenCode / DeepSeek V4.1 Flash 1017）

## 分支与提交

- Worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/opencode-script-runner`
- 分支：`codex/feature-script-runner`
- 代码提交：`51874d5 feat(script-runner): implement module 06 custom script library and execution`
- 未合并主分支、未推送远端。HANDOFF 本身为后续独立提交。

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
  `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` 隐藏控制台。
- `on_init` 只标记上次残留的 running 记录并注册已有快捷键，不执行脚本；
  `script_save` 只持久化，不执行脚本。仅在 `script_execute`、单脚本快捷键或托盘点击时运行。
- 停用/退出时终止活动子进程、等待监控线程收尾、注销脚本快捷键、清理临时运行目录。
- 单脚本快捷键使用 `script:run:{id}`，接入现有 `ShortcutRegistry` 生命周期；保存/删除后由
  Tauri 重新同步全局快捷键并重建托盘“快捷脚本”平铺菜单。
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

## 真实验证结果

最终验证均在当前 worktree 执行，未使用 shell 管道掩盖测试退出码。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出码 0 |
| `TMPDIR="$PWD/.runtime_data/test-tmp/tmp" cargo test --workspace --offline --all-targets` | 退出码 0；`mod-script-runner` 16 passed；host `tests/modules` 8 passed；workspace 其余全部通过；1 个原生窗口枚举测试按设计 ignored |
| `pnpm --dir src-ui test` | 退出码 0；2 passed，0 failed（三语键结构一致、Logo 矢量结构） |
| `pnpm --dir src-ui run build` | 退出码 0；`vue-tsc --noEmit` 通过，Vite 生产构建成功 |

`mod-script-runner` 的 16 个测试覆盖：

- 七种脚本类型的 argv 构造，包含 PowerShell/Batch/VBScript 的独立参数与路径空格；
- 内联/文件脚本 CRUD、重启持久化、初始化与保存不执行；
- 真实 Shell stdout/stderr/退出码；
- 真实 Python、Node、AppleScript 执行（解释器不可用时显式跳过并记录原因）；
- 带空格/引号参数、文件路径空格、工作目录、原文件不被改写；
- 睡眠脚本超时终止、取消、停用清理；
- 增量日志序号、日志缓存上限、final 模式抑制实时条目；
- `AIO_CLIPBOARD`/`AIO_SELECTION` 注入与 Unavailable 时不设置；
- 单脚本快捷键冲突、启停注册/注销、删除后保留历史记录。

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
2. **Windows 仅做编译期条件与 argv 单测，未在 Windows 实机执行**：控制台隐藏标志与
   PowerShell/Batch/VBScript 命令构造已覆盖，但未实机验证。
3. **Windows 取消只 `Child::kill` 直接子进程**：Unix 使用进程组连同子进程终止；Windows 未实现
   Job Object 级联终止，若脚本自身派生后台进程可能残留。需后续补 Job Object 或 taskkill /T。
4. **OS 级快捷键注册失败未回滚脚本保存**：模块注册表会登记快捷键，Tauri 会尝试重新注册；
   若系统级注册失败，当前仅记录错误并保留已保存脚本。集成时建议补充可见错误或回滚策略。
5. **托盘快捷键菜单为平铺项，不是嵌套 Submenu**：行为与 `show_in_tray` 一致，菜单项点击即执行；
   未使用子菜单层级。
6. **`output_handling` 的 `final` 与 `discard` 目前都抑制实时事件**，`final` 保留最终尾部日志；
   `discard` 也不清空最终尾部（界面文案为“不推送实时日志”）。若需彻底丢弃输出，需再调整。
7. **GUI 未执行**：不能据此提交发布门槛通过结论。
8. 公共文件存在并行成员改动风险，冲突由 Codex 集成处理；本 worktree 未修改其他成员目录。
