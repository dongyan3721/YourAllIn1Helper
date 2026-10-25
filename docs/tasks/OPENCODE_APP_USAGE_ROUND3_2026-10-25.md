# OpenCode：功能 05 使用时长统计，第三轮（锁屏/睡眠事件 + 音频 AFK 例外）

协调者：Cursor Agent / Claude Opus 5.5（接替 OpenAI Codex 统筹）。执行者模型保持 DeepSeek V4.1 Flash 1017（`deepseek/deepseek-flash`），以 `opencode run --auto` 启动；不修改全局权限/代理，不购买额度，不更换模型。

## 工作区

- 专用 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/opencode-app-usage`
- 分支：`codex/feature-app-usage`；基线为已合入第二轮集成的本地 master（本任务书所在提交）。
- 先执行 `pwd`、`git branch --show-current`、`git rev-parse HEAD` 并报告。所有写入只在本 worktree；不改主目录、其他成员目录，不 merge/push。
- 使用本 worktree 独立 `target/`（`CARGO_TARGET_DIR=$PWD/target`）与 `src-ui/node_modules`，避免与其他成员争用。

## 先读

`docs/handoffs/app-usage.md`（含第二轮审查修正）、`docs/testing/ROUND2_INTEGRATION_REPORT_2026-10-25.md`、`docs/testing/GUI_TEST_APP_USAGE.md`、`docs/MODULE_ARCHITECTURE.md`、`PROJECT_PLAN.md` 5.5。

## 本轮实际开发内容

1. **macOS 锁屏 / 息屏 / 睡眠改为事件驱动**
   - 第二轮实测：CLI/测试进程取不到 `CGSessionCopyCurrentDictionary` 会话字典，当前只能轮询且失败即暂停。保留该严格失败语义。
   - 在宿主 AppKit 进程订阅真实系统通知：`NSWorkspace` 的 `willSleep` / `didWake` / `screensDidSleep` / `screensDidWake`，以及分布式通知 `com.apple.screenIsLocked` / `com.apple.screenIsUnlocked`。平台代码放 `os-hal` 独立文件或宿主独立文件；业务模块只依赖 trait / 事件，不依赖 objc2。
   - 事件状态与轮询结果合并：任一来源报告锁定/睡眠即停止计时；唤醒后先确认解锁再恢复；未知状态保持暂停，不当作活跃。
   - 睡眠跳变与单调时钟逻辑不能回退：睡眠期间不计入时长，唤醒后不补算。
2. **音频活动 AFK 例外（真实能力）**
   - macOS 使用 CoreAudio 默认输出设备的 `kAudioDevicePropertyDeviceIsRunningSomewhere`（无需额外权限），抽象为 HAL 能力（例如 `AudioActivityProbe` 或在 `ActiveAppWatcher` 增加带默认 `Unsupported` 的方法）。
   - 规则：键鼠空闲超过阈值，但默认输出设备正在播放时，不进入 AFK（视频/音乐场景）；锁屏/睡眠优先于音频例外。设置项可关闭（默认开启），三语界面显示真实能力状态；Windows 未实现时返回 `Unsupported` 并显示限制，不返回假值。
3. **会话列表 UI**：`/usage` 面板补充当日会话列表（应用、开始/结束、时长；标题按隐私设置显示），空状态、分页或上限。
4. 修订 `docs/testing/GUI_TEST_APP_USAGE.md`：锁屏、息屏、睡眠唤醒、播放音频挂机、关闭音频例外各列一条可执行用例。

## 约束

- 业务、配置、测试分文件，新增代码中文注释；真实 HAL，生产禁止 Mock 回退；`MockOsHal` 只用于显式测试。
- 注入时间源 / 事件源编写有意义测试：锁屏期间不计时、唤醒不补算、音频播放时空闲不暂停、锁屏优先于音频、能力失败暂停、开关持久化、停用释放通知观察者与线程。
- 停用模块必须移除系统通知观察者并停止线程；重复启停不泄漏观察者。
- 不采集按键内容；测试只用隔离数据，不开启日常应用后台统计；不播放用户媒体、不改系统音量。
- 只做本功能所需公共接线，在交接中列出共享文件；不触碰截图、翻译、密码本目录，不改剪贴板公共层（协调者正在实现共享剪贴板租约）。
- 允许从 crates.io 官方源新增必要依赖（如 `objc2-core-audio` 或现有 objc2 系列 feature），在交接中写明用途与版本。

## 验收与交接

- `cargo fmt --all -- --check`、`cargo test -p mod-app-usage --offline`、`cargo test --workspace --offline --all-targets`、`pnpm --dir src-ui run build`、`pnpm --dir src-ui test`，如实记录结果。
- 需要桌面会话的原生检查可 `#[ignore]` 并写明显式运行方式；不得弱化断言冒充通过。
- GUI 用例未实际操作就标未执行（实体锁屏/睡眠/音频由协调者在 GUI 验收阶段执行），不能以 IPC 替代。
- 分批 commit，正文记录 `Agent: OpenCode` 与 `Model: DeepSeek V4.1 Flash 1017`。
- 更新 `docs/handoffs/app-usage.md` 新增「第三轮」章节：提交、接口/配置、公共文件清单、真实测试结果、未执行项与平台限制。

先报告实际路径、分支、模型与步骤，然后立即编码。阻塞时报告具体原因，不伪报进度。
