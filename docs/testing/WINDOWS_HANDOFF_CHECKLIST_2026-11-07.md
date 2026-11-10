# Windows 待测与 Cursor 交接清单

更新时间：2026-11-10。下列未勾选项均未完成；不要把普通单测通过等同于真实桌面或硬件验收通过。

## 当前状态与新增验收

- **2026-11-08 GUI 第二轮**：GIF 黑屏、长截图蓝框、原生另存为及默认保存、Windows 无衬线字体已修复并完成主要实测；最新自动化为 Rust 337 passed / 16 ignored、前端 85 passed。详见 [最终集成报告](WINDOWS_GUI_ROUND2_2026-11-08.md)。
- IDE 候选窗焦点已增加原生前台确认、WebView2 内部交焦保护和窗口级按键处理；2026-11-10 CodeBuddy 呼出、Down、Enter 粘贴、Ctrl+S 后磁盘文本严格核对均通过，详见 [补充验收](WINDOWS_GUI_FOLLOWUP_2026-11-10.md)。WorkBuddy 本体及管理员权限目标仍待测。
- 2026-11-10 后续补丁：翻译气泡工作区/原生外框约束、GIF 排除登记变更时停止和超宽标识拒绝已集成，完整 Rust 回归为 349 passed / 16 ignored；真实窗口边界检查通过。分工及剩余边界见 [后续报告](../WINDOWS_FOLLOWUP_COORDINATION_2026-11-10.md)。
- 旧 C 盘四个 worktree、本轮 D 盘 scroll/save/focus 及后续 gif/capture-review/translator-bounds 均已清理。gif 的未提交原件、diff 已逐文件核对后归档到 D 盘；Git worktree 只剩主目录，Cursor 占用的空目录保留。以下分支/worktree 描述属于上一轮现场记录。

### 上一轮验收记录

- 六个本地开发分支的提交已全部包含在 `master`；四个独立 worktree 没有未提交内容。
- Windows Hello 和主导航滚动条已修复，真人往返与主界面确认通过，详见 [2026-11-08 修复报告](WINDOWS_HELLO_FIX_2026-11-08.md)。最近完整 Rust 工作区为 302 passed、0 failed、16 ignored，前端 73 passed。
- 用户于 2026-11-08 确认已测试剪贴板功能，除下述 IDE 焦点问题外未发现其他问题，同意提交现有图片剪贴板修复。
- **2026-11-08 前一轮已知问题（第二轮已修复，扩展验收未完成）**：用户在 WorkBuddy 中呼出候选窗后方向键仍落在 IDE。第二轮 CodeBuddy 候选窗和方向键已通过，完整存盘、WorkBuddy 与管理员目标仍见上述待测边界；原图片剪贴板提交本身不包含该修复。
- 全屏 RGBA 剪贴板原生专项仍为显式 ignored，未单独重跑；位图编码单测已随上述完整工作区测试通过。

## 2026-11-07 接手现场（历史记录）

- **接手主目录**：`C:\Users\xcxix\PyCharmProjects\YourAllIn1Helper`；当前分支 **`codex/windows-integration`**，HEAD **`cdf1100`**。四个外部智能体的实现与后续修复已合入此分支；**没有合入 master，没有向远程推送**。`master` 仍为 `5df7291`。
- Cursor 原来的 `YourAllIn1Helper-windows-vault` worktree 仅包含其密码本分支，不是完整集成代码。接任集成时请使用上述主目录；保留其他分支和 worktree。
- 交接时存在未提交改动：`README.md`、`crates/os-hal/tests/capabilities.rs`（增加 Unsupported 原因诊断），以及本清单和 `WINDOWS_INTEGRATION_2026-11-07.md`。先读 diff，保留现场，不要 reset/clean。
- 测试日志和虚构测试数据在忽略目录 `.runtime_data/windows-validation/`。
- 交接时运行的是 **QA 副本 PID 14536**：`.runtime_data/windows-validation/merged-app/all-in-one-desktop.exe`，使用隔离数据目录 `merged-gui-data`。该副本早于最后 UIA 修复，不能代表最终 exe；仍占用全局快捷键。后续测试前核对 PID 与绝对路径，再正常退出该实例，避免双实例快捷键冲突。
- 最新主目录产物：`target/debug/all-in-one-desktop.exe`，已包含 `cdf1100` 的 UIA 修复和最新界面，尚未作为普通实例启动。不要同时运行旧 QA 副本与新版。
- Rust 命令所在 shell 若找不到 cargo，先执行 `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"`。Rust 1.99.1/MSVC、VS Build Tools/SDK、WebView2、Node 与 pnpm 已具备；并发限制为 2。

## 已有证据

- [x] 最后一次完整 Rust 工作区：**296 passed、0 failed、13 ignored**，32 个测试程序。日志 `cursor-workspace-tests.log`。
- [x] 前端：**73 passed**；TypeScript/Vite 与 Windows 调试 exe 构建通过。日志 `cursor-ui-tests.log`。
- [x] 原生凭据/敏感剪贴板/GIF 5 项，以及关闭后被抢占的剪贴板回归 2 项通过。日志 `integration-native-secure.log`、`integration-native-clipboard-race.log`。
- [x] 原生 WTS 状态、音频查询、事件监视器启停 3 项通过；不等于真实锁屏、睡眠、电源或音频热插拔验收。
- [x] 用户在 CodeBuddy 触发 Ctrl+Shift+V、点击历史条目并保存，确认跨应用粘贴成功；磁盘 `paste-target.txt` 已核对重复 QA 文本。自动化点击瞬态弹窗时会失焦，未将工具点击失败直接判为产品失败。
- [x] 最终 exe 的 CLI 烟测与原生 UI 烟测退出码都是 0。日志 `cursor-cli-smoke-final.log`、`cursor-ui-smoke-final.log`。

## P0：先解决再结束集成

- [x] **UIAutomation 原生正例**。根因是采样点被其他顶层窗口挡住，以及只暴露 HWND 根的 provider 被误判为父链不完整。可见记事本与置顶 CodeBuddy 均为 5/5；遮挡不再算作目标应用成功。没有把全 Unsupported 改成通过，也没有补造父节点。窗口边缘、混合 DPI 仍见 P1。
- [x] 已结束隔离 QA 副本 PID 14536（路径为 `merged-app\all-in-one-desktop.exe`）。最终构建 `--smoke-test` 与 `--ui-smoke-test` 的进程退出码都是 0。UI 烟测使用程序内的临时数据目录。
- [x] 修复后已重跑工作区测试、前端测试和 `pnpm build:desktop`。`git diff --check` 通过，集成报告已更新。
- [x] 集成分支已合入本地 master（`8d04acb`）。重新执行 `pnpm build:desktop` 后启动普通实例，总览列出 10 个已开启模块，并显示已注册的全局快捷键（含 Ctrl+Shift+V、F1、Alt+F1）。没有向远程推送。`cargo test --workspace` 会把调试 exe 覆盖成指向 localhost 的开发构建，普通启动前需要再跑一次 `pnpm build:desktop`。

## P1：本轮新迁移能力的真实验收

| 待测项 | 操作及通过条件 |
| --- | --- |
| [ ] Windows Hello 扩展硬件验收 | 2026-11-08 已通过真实 NGC 登记、两次私钥解密、撤销及主界面启用/解锁，硬件用例已替换为真实往返测试。进程重启后解锁、取消/失败、策略拒绝/提供程序不可用仍需专项覆盖；主密码必须保留 |
| [ ] 密码本锁定观察者 | 密码本已解锁时真实锁屏、睡眠、会话断开/注销，确认自动锁定；恢复后不自动解锁。验证重复启停、退出无遗留 observer |
| [ ] 敏感复制端到端 | 用虚构秘密从密码本 UI 复制；核对可以粘贴，约 30 秒后只清除仍属本次写入的内容；期间用户另复制不同文本或相同文本均不能被误清除。再测正常退出提前清除、写入失败和快速连续复制 |
| [ ] 系统历史与云同步排除 | 在用户现有配置下手工确认虚构秘密不进入 Windows 剪贴板历史或跨设备同步；不为测试擅自变更隐私设置。代码和格式测试通过不能替代此项 |
| [ ] Credential Manager 持久性 | 虚构命名空间的读写删和越界服务名拒绝已通过。进程重启后的持久性、空值和二进制边界还没单独复测 |
| [ ] 使用统计真实状态变化 | 前台应用查询、空闲秒数和事件监视器启停通过。锁屏、睡眠、显示器开关和热插拔还没做 |
| [ ] Core Audio 真实播放与热插拔 | 默认输出设备查询返回了真实布尔值。静音、播放、暂停和拔插还没做 |
| [ ] 物理 Ctrl+C/V 失败边界 | 外部目标关闭、焦点被其他窗口抢走、HWND 失效、应用自身前台、持续按住修饰键、提权目标，确认取消或明确报错，不向错误窗口发送按键；补测普通输入框和多应用 |
| [ ] UIA 多屏/DPI | 100%/150%/200% 混合缩放、负坐标副屏、跨屏窗口、最小化/遮挡窗口；截图吸附区域与光标及真实控件一致，父链缩放正确 |
| [ ] GIF 目标兼容 | 虚构 GIF 已写入 `image/gif` 和 `GIF` 格式。还没向多个实际目标粘贴确认动画 |

## P2：Windows 整体回归与交付

- [ ] 截图：区域/窗口捕获、冻结帧、贴图、OCR、滚动截图；多屏与高 DPI 下坐标正确，Esc 取消及重复开关无残留覆盖层。
- [ ] 便笺与窗口置顶：保存后退出/重启恢复，关闭/创建、快捷键、目标窗口退出处理正确。
- [ ] 屏幕注释：Ctrl+Shift+D 可以开启和退出。保存快照已能写入全屏位图和 PNG，不再出现剪贴板 1418。绘制和撤销还没单独走完。
- [ ] 脚本执行：PowerShell 5.1/7、BAT、Python 的中文/空格路径、取消、超时、缺失解释器；使用无副作用测试脚本。
- [ ] 翻译：真实划词/复制触发、网络成功/失败/超时；自备有效服务配置，不把缺少凭据当作迁移成功。
- [ ] 媒体处理：FFmpeg 已安装/缺失、中文及空格路径、取消、输出文件正常；仅使用测试媒体。
- [ ] 应用生命周期：首次启动、普通退出、托盘隐藏/恢复、快捷键冲突、模块反复启停、异常后再启动、数据目录持久化。
- [ ] 安装交付：release 构建、Windows 安装包安装/升级/卸载、普通用户与新账户、WebView2 缺失提示；**目前只确认调试 exe，不代表安装交付已验证**。
- [ ] 跨平台边界：macOS 编译和桌面回归；本机未测，不能宣称保留代码就已验证兼容。

## 建议复测命令

在上述主目录执行；原生 clipboard 专项会覆盖系统剪贴板，安排在用户不依赖当前内容时进行。Hello 真人测试单独交由用户操作，不一揽子运行所有 crate 的 ignored 测试。

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cargo test --workspace --locked --all-targets -j2
pnpm --dir src-ui test
pnpm build:desktop

# 保持自备 QA Notepad 可见，再运行正例；勿依赖旧测试 exe 的文件名。
$env:AIO_UIA_SMOKE_PROCESS = 'notepad'
cargo test -p os-hal --locked --test capabilities -- --ignored --test-threads=1 --nocapture

cargo test -p os-hal --locked --test secure_services -- --ignored --test-threads=1 --nocapture
cargo test -p os-hal --locked --lib preemption -- --ignored --test-threads=1 --nocapture
```

详细实现及局限见 [集成报告](WINDOWS_INTEGRATION_2026-11-07.md) 和四个专项报告。第三方 UIA provider 的同步 COM 调用仍可能阻塞，遍历深度限制不能取消单次挂死调用；崩溃/强杀后秘密定时清理也无法保证执行。
