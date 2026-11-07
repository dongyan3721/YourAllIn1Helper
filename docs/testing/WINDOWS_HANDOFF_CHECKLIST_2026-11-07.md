# Windows 待测与 Cursor 交接清单

更新时间：2026-11-07。下列未勾选项均未完成；不要把普通单测通过等同于真实桌面或硬件验收通过。

## 接手位置与现场

- **接手主目录**：`C:\Users\xcxix\PyCharmProjects\YourAllIn1Helper`；当前分支 **`codex/windows-integration`**，HEAD **`cdf1100`**。四个外部智能体的实现与后续修复已合入此分支；**没有合入 master，没有向远程推送**。`master` 仍为 `5df7291`。
- Cursor 原来的 `YourAllIn1Helper-windows-vault` worktree 仅包含其密码本分支，不是完整集成代码。接任集成时请使用上述主目录；保留其他分支和 worktree。
- 交接时存在未提交改动：`README.md`、`crates/os-hal/tests/capabilities.rs`（增加 Unsupported 原因诊断），以及本清单和 `WINDOWS_INTEGRATION_2026-11-07.md`。先读 diff，保留现场，不要 reset/clean。
- 测试日志和虚构测试数据在忽略目录 `.runtime_data/windows-validation/`。
- 交接时运行的是 **QA 副本 PID 14536**：`.runtime_data/windows-validation/merged-app/all-in-one-desktop.exe`，使用隔离数据目录 `merged-gui-data`。该副本早于最后 UIA 修复，不能代表最终 exe；仍占用全局快捷键。后续测试前核对 PID 与绝对路径，再正常退出该实例，避免双实例快捷键冲突。
- 最新主目录产物：`target/debug/all-in-one-desktop.exe`，已包含 `cdf1100` 的 UIA 修复和最新界面，尚未作为普通实例启动。不要同时运行旧 QA 副本与新版。
- Rust 命令所在 shell 若找不到 cargo，先执行 `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"`。Rust 1.99.1/MSVC、VS Build Tools/SDK、WebView2、Node 与 pnpm 已具备；并发限制为 2。

## 已有证据

- [x] 最后一次完整 Rust 工作区：**295 passed、0 failed、13 ignored**，32 个测试程序。日志 `integration-workspace-tests-final.log`。
- [x] 前端：**73 passed**；TypeScript/Vite 与 Windows 调试 exe 构建通过。日志 `integration-ui-tests-final.log`、`integration-desktop-build-final.log`。
- [x] 原生凭据/敏感剪贴板/GIF 5 项，以及关闭后被抢占的剪贴板回归 2 项通过。日志 `integration-native-secure.log`、`integration-native-clipboard-race.log`。
- [x] 原生 WTS 状态、音频查询、事件监视器启停 3 项通过；不等于真实锁屏、睡眠、电源或音频热插拔验收。
- [x] 用户在 CodeBuddy 触发 Ctrl+Shift+V、点击历史条目并保存，确认跨应用粘贴成功；磁盘 `paste-target.txt` 已核对重复 QA 文本。自动化点击瞬态弹窗时会失焦，未将工具点击失败直接判为产品失败。
- [x] 较早版本 CLI 5 步烟测、原生 UI 烟测退出码 0。最后 UIA 合入后的最终 exe 仍需重跑。

## P0：先解决再结束集成

- [ ] **UIAutomation 原生正例失败**。最终集成构建在 CodeBuddy、现代 Notepad 均为 **0/5** 有效控件链，诊断原因 `windows_uiautomation_incomplete_ancestry`。Zed 开发分支曾报告 Notepad 5/5，但集成环境没有复现。保留失败断言，不可改成全 Unsupported 也通过。
  - 相关实现：`crates/os-hal/src/platform/windows/regions.rs`；入口测试：`crates/os-hal/tests/capabilities.rs` 的 `windows_ui_hierarchy_returns_real_chain_for_requested_qa_process`。
  - 失败日志：`integration-native-capabilities-final.log`、`integration-native-capabilities-notepad-final.log`、`integration-uia-diagnostic.log`。
  - 最后诊断版测试程序 `capabilities-135f0459376d1d64.exe` 已构建，单独运行仍失败。下一轮优先核对命中 HWND/进程、遮挡与 DPI 坐标、ElementFromHandle 根节点与 Raw View 父链身份；这些只是排查方向，尚未证明根因。
  - 验收：可见 Notepad 正例返回真实叶子到目标 HWND 根节点的链；ID 父子关系正确，矩形落在真实窗口范围内。再测 Electron 应用、窗口边缘、应用自身覆盖层及不支持的 provider；不得补造父节点。
- [ ] 停止旧 QA 实例，用最终构建重跑 `--smoke-test`、`--ui-smoke-test`，分别记录进程退出码。UI 烟测用独立数据目录；PowerShell 直接重定向 GUI 进程曾遇到管道不结束，宜使用 Start-Process 的文件重定向与显式 WaitForExit。
- [ ] 修复后重跑相关测试、完整 Rust/前端测试和最终桌面构建；检查 `git diff --check`，更新集成报告，提交未提交文件。
- [ ] 通过后再将集成分支合入本地 master，启动最终普通实例，确认模块、托盘和快捷键可用；远程推送不在本轮授权范围内。

## P1：本轮新迁移能力的真实验收

| 待测项 | 操作及通过条件 |
| --- | --- |
| [ ] Windows Hello 实机完整往返 | 用户亲自完成认证：新建测试密码本、启用设备密钥、锁定、Hello 解锁、重启后再解锁；取消和认证失败不解锁。测试策略拒绝/提供程序不可用时明确回退主密码。当前只测过配置、错误与失败边界，**没有执行真实 NGC 密钥及手势私钥解密**；现有 ignored 硬件用例是占位，不能算往返测试 |
| [ ] 密码本锁定观察者 | 密码本已解锁时真实锁屏、睡眠、会话断开/注销，确认自动锁定；恢复后不自动解锁。验证重复启停、退出无遗留 observer |
| [ ] 敏感复制端到端 | 用虚构秘密从密码本 UI 复制；核对可以粘贴，约 30 秒后只清除仍属本次写入的内容；期间用户另复制不同文本或相同文本均不能被误清除。再测正常退出提前清除、写入失败和快速连续复制 |
| [ ] 系统历史与云同步排除 | 在用户现有配置下手工确认虚构秘密不进入 Windows 剪贴板历史或跨设备同步；不为测试擅自变更隐私设置。代码和格式测试通过不能替代此项 |
| [ ] Credential Manager 持久性 | 用应用专用虚构条目验证进程重启后的读取、覆盖、删除、空/二进制值及边界错误；禁止读写用户其他真实凭据 |
| [ ] 使用统计真实状态变化 | 前台应用切换、空闲、锁屏/解锁、断开/重连、睡眠/唤醒和显示器关闭/恢复，核对计时暂停与恢复、不重复累计。含 Modern Standby/合盖设备时单独验证 |
| [ ] Core Audio 真实播放与热插拔 | 静音、实际播放、暂停、无默认端点、切换输出设备/拔插，确认活动状态和错误上报；现有测试仅证明一次系统查询可成功 |
| [ ] 物理 Ctrl+C/V 失败边界 | 外部目标关闭、焦点被其他窗口抢走、HWND 失效、应用自身前台、持续按住修饰键、提权目标，确认取消或明确报错，不向错误窗口发送按键；补测普通输入框和多应用 |
| [ ] UIA 多屏/DPI | 100%/150%/200% 混合缩放、负坐标副屏、跨屏窗口、最小化/遮挡窗口；截图吸附区域与光标及真实控件一致，父链缩放正确 |
| [ ] GIF 目标兼容 | 用虚构短动画向多个实际目标粘贴，确认保持动画或明确不支持；当前仅写 image/gif/GIF，无 CF_DIB 静态回退 |

## P2：Windows 整体回归与交付

- [ ] 截图：区域/窗口捕获、冻结帧、贴图、OCR、滚动截图；多屏与高 DPI 下坐标正确，Esc 取消及重复开关无残留覆盖层。
- [ ] 便笺与窗口置顶：保存后退出/重启恢复，关闭/创建、快捷键、目标窗口退出处理正确。
- [ ] 屏幕注释：开启、绘制、撤销、退出，覆盖层及快捷键完整释放。
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
