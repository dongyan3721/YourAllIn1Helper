# Windows 并行迁移与集成验收（2026-11-07）

本轮在 Windows x64 上由四个已打开的外部编辑器智能体开发，Codex 负责分工、独立 worktree、预审、冲突处理、集成与复测。基线是 `c7c6a4d`，环境搭建见 [首次启动报告](WINDOWS_STARTUP_2026-11-07.md)。没有向远程推送。

## 分支与交付

| 开发者 | 分支 | 交付 | 分支顶端 |
| --- | --- | --- | --- |
| Cursor / Grok | `codex/windows-vault` | Hello 设备密钥与密码本自身会话/睡眠观察者 | `6c342ee` |
| Antigravity / Gemini | `codex/windows-usage` | WTS 锁屏轮询、会话/显示电源/睡眠事件、Core Audio | `cd00df4` |
| CodeBuddy / MiMo | `codex/windows-secure-services` | Credential Manager、敏感剪贴板、GIF、所有权竞态回归 | `72ff165` |
| Zed / Copilot | `codex/windows-input-regions` | 外部 HWND/PID 粘贴目标、物理快捷键、UIA 点命中和真实父链 | `73d73f6` |

各分支均独立提交，保留 worktree。详细记录：[密码本](WINDOWS_VAULT_2026-11-07.md)、[使用状态](WINDOWS_USAGE_2026-11-07.md)、[安全服务](WINDOWS_SECURE_SERVICES_2026-11-07.md)、[输入与区域](WINDOWS_INPUT_REGIONS_2026-11-07.md)。分支报告描述各自提交时的状态；以下集成修正覆盖其中已过时的跨模块待办。

## 集成修正

- 合并 Windows feature 增量、能力测试与三语文案冲突，保留平台边界和原有 macOS 实现。没有升级锁定的依赖版本；Cargo.lock 只增加密码本对既有 Windows 依赖的引用。
- 预审发现并修正 UIA 的 `S_OK + NULL` 误判，以及释放 COM 身份引用后只比较裸地址的问题；父链只有真实抵达 HWND 根节点才返回，不能补造窗口父级。
- 原生验收发现现代应用的过滤父链不可用，退回修复后采用 `ElementFromPoint` 和 Raw View 父链；Control View 子树只作有界回退。保留 32 层与 1024 节点上限。
- 发送 Ctrl+C/V 前等待物理修饰键释放，并再次校验前台 HWND/PID。失效、自身窗口、焦点恢复失败时拒绝输入，不绕过 Windows 前台限制。
- 使用 SDK 的显示电源 GUID，替换错误的手写值；按 DWORD 读取通知载荷。WTS 已断开的会话暂停计时；音频查询错误不再全部吞成 false。
- 观察者的注册、清理、停止与重启经过审查；密码本观察者增加两阶段启动握手，启动接收端退出或超时后自行释放窗口和服务引用。
- 修正 Windows 剪贴板排除格式名、有效 owner HWND、GlobalUnlock 的 FALSE/last-error 语义。写入关闭后重新独占打开，验证存活 owner 和本次唯一标记后才采最终序号；不得认领或清除抢占者的内容。
- 密码本实际复制入口接到 `ClipboardOwnership`。计时器只持有不含明文的凭证，租约覆盖秘密停留窗口；写入失败释放租约，正常退出提前按所有权清除。普通无桌面宿主不自动使用真实剪贴板，也不以生产 Mock 冒充成功。
- 凭据服务名按应用命名空间边界限制，拒绝形似前缀的其他服务；私有剪贴板标记读取限制为 8 字节，注册或写入标记失败在原事务内回滚。
- 界面按真实设备密钥种类显示 Windows Hello，并保留通用平台文案；校正既有 Windows OCR 说明。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace --locked --all-targets -j 2` | 32 个测试程序；296 通过、0 失败、13 项默认忽略。日志 `cursor-workspace-tests.log` |
| `pnpm --dir src-ui test` | 73 通过、0 失败。日志 `cursor-ui-tests.log` |
| 前端 TypeScript 与 Vite 构建 | 通过 |
| `pnpm build:desktop` | UIA 修复后的 Windows x64 调试版重新构建通过，界面资源嵌入 exe |
| CLI 烟测 | 最终 exe `--smoke-test` 5 步通过，进程退出码 0。日志 `cursor-cli-smoke-final.log` |
| 原生 UI 烟测 | 最终 exe `--ui-smoke-test` 通过，进程退出码 0。日志 `cursor-ui-smoke-final.log` |
| WTS/音频/事件监视器专项 | 3 项真实原生测试通过 |
| 凭据、敏感剪贴板、GIF 专项 | 5 项通过；只使用应用专用虚构凭据与文本 |
| 关闭后被抢占的剪贴板回归 | 2 项确定性原生测试通过，包含后来复制完全相同文本 |
| UIAutomation 正例 | 记事本与 CodeBuddy 在真正位于采样点最上层时均为 **5/5** 条控件链，矩形包含采样点，父子 ID 连续。此前 0/5 是全屏 Zed 等窗口挡住采样点，查询落到了别的窗口 |
| 跨应用物理粘贴 | 用户在 CodeBuddy 中触发 Ctrl+Shift+V、点击历史项并保存后确认成功；磁盘文件中已核对出现重复的 `Windows migration clipboard QA` 文本 |

首次全量构建遇到旧托盘实例占用 exe，停止该开发实例后重跑通过。一次 UI 烟测的 PowerShell 管道未结束，已改为显式等待进程并确认退出码 0。

## Cursor 接手后的 UIA 复测

交接时的 0/5 不是记事本或 CodeBuddy 的父链断了。本机 Zed 窗口是 1920×1032 的全屏窗口，采样点实际命中它。Zed 的 UIAutomation 只暴露 `Zed::Window` 这一个根元素，`CompareElements` 确认它就是 `ElementFromHandle` 的根节点，上面没有子控件。旧逻辑把“已经到达根、但子链为空”报成 `windows_uiautomation_incomplete_ancestry`。

现在分两种真实结果：

- 父链能从叶子走到目标 HWND 根节点时，返回裁剪后的控件链。可见记事本 5/5，置顶后的 CodeBuddy（Electron）5/5，本应用 WebView2 窗口在置顶时也能走通到 `Tauri Window`。
- 点命中的元素就是 HWND 根节点时，只返回该真实窗口，不补造子控件或父节点。Zed 属于这一类。
- 采样点被其他顶层窗口挡住时，正例测试记为遮挡并失败，不再把挡住它的窗口算成目标应用的控件链。用工具箱盖住记事本后，5 个点都报告被 `Tauri Window` 挡住。

`git diff --check` 通过。旧 QA 副本 PID 14536 路径确认为 `.runtime_data/windows-validation/merged-app/all-in-one-desktop.exe`，关闭主窗口后仍驻留托盘，已结束该隔离副本。本机缩放为 100%（96 DPI），混合缩放和副屏仍未测。

本机日志位于忽略目录 `.runtime_data/windows-validation/`。保留平台已有未使用项/不可达分支警告及 Vite 大分块提示；没有扩大范围执行自动清理。

## 边界与人工验收

- Hello 只验证了配置/提供程序可用性、错误映射、信封与失败边界。没有创建真实 NGC 设备密钥、执行真人手势或非静默私钥解密。被忽略的硬件用例是占位说明，不能当作实际硬件往返测试。策略不支持必须失败，主密码始终保留；PIN 不等同于生物识别集合绑定。
- 真实锁屏、睡眠/合盖、Modern Standby、显示器电源切换及音频端点热插拔仍需人工硬件验收；普通单测只向本进程观察窗口注入消息。
- UIA 的第三方 provider 可能不响应同步 COM 调用。深度/节点限制不能取消单次挂死的 provider；本轮不承诺覆盖所有应用。Zed 这类只暴露窗口根的 provider 得到真实窗口矩形，没有子控件链。多显示器负坐标有几何回归，真实混合 DPI、多桌面与提权应用仍需专项验收。本机复测时缩放为 100%。
- GIF 只写动画格式，没有 CF_DIB 静态回退；目标应用能否粘贴动画需逐项验收。剪贴板历史与云同步按微软标记排除，真实系统历史/跨设备效果未人工验证。
- 敏感标记探测保守跳过任何包含历史/同步控制格式的内容，即使其 DWORD 为 1；可能少收普通内容。剪贴板标记不是防御同用户恶意进程的权限边界。进程被强制终止或系统崩溃时，定时清理无法保证执行。
- macOS 代码边界保留，但本机没有进行 macOS 编译或桌面回归。FFmpeg、解释器和翻译网络服务继续是可选运行依赖。

Cursor 接手后已将集成分支合入本地 master（`8d04acb`），没有向远程推送。普通实例的总览能看到 10 个模块和已注册快捷键。P1/P2 的硬件与安装包验收仍未做，见 [Windows 待测与交接清单](WINDOWS_HANDOFF_CHECKLIST_2026-11-07.md)。
