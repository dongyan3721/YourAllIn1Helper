# 第二轮集成与验收报告（2026-10-25）

## 交付结论
01 截图 M1、05 使用时长 M1+M2、07 翻译 M1 已集成，当前挂载 01–09 九个模块。10 密码本只交付 M0 设计，无可运行代码，仍显示待开发。M0 审查通过的约束见 [审查意见](../design/password-vault-m0-review.md)，不能据此宣称密码本或生物识别已完成。

本轮已完成代码审查、冲突处理、审查缺陷修复、工作区测试、严格前端构建、release App、原生烟测及部分实际 GUI 验收。GUI 受权限和凭据限制的流程保持待测，不以自动化测试替代人工闭环。

## 来源与修正
保留成员原始提交及分支：
| 来源 | 原始提交 | 后续审查修正 |
| --- | --- | --- |
| Cursor / Claude Opus 5.5：截图 | 50dd5a5 | 0c0f5bf、53e1900 |
| OpenCode / DeepSeek V4.1 Flash 1017：统计 | 8707006 | f9063be |
| Antigravity / Gemini 3.8 Flash：翻译 | 9cca0d3 | 8122ecb |
| WorkBuddy / Kimi K3.1 Pro：密码本设计 | 6556a36 | ca4dd27 设计审查 |
| OpenAI Codex：集成/验收 | 集成分支 codex/integrate-remaining-features | eb393ac 最终 GUI 文案及目录测试修正 |

修复的主要问题：
- 截图原生关闭回调同步等待会话锁，与正在等待窗口操作的导出任务形成死锁；改为阻止默认关闭、后台按会话 ID 取消。配置中的颜色/线宽实际生效。
- 宿主统一异步退出协调，先注销快捷键，再后台排空模块；保持 AppKit 可处理窗口销毁，重复退出请求合并。
- 使用统计关闭标题采集后会保留旧标题、持久化失败可能丢批次、墙上时钟跳变可能虚增时长；修正隐私清理、原子事务与重试、单调计时和本地小时切段。会话查询失败暂停计费。停用失败有明确恢复钩子，恢复失败保持禁用。
- 翻译 reqwest 未启用 TLS，HTTPS 无法工作；启用 native-tls，并用隔离 TLS 服务验证。修正浮窗 ready 事件丢失、先抢焦点再读选区、同步回调中 tokio::spawn、请求取消/迟到结果、SSE DONE 后挂起等问题。普通测试不读取真实 AX 选区或依赖用户本机 Ollama。
- GUI 修复统计空状态显示字典键、翻译空结果误显示“翻译中”；中英日字典同步。尚未实现的自动复制/历史设置明确禁用。

## 构建与环境
- macOS 27.0，arm64。
- 最终生产源码提交：eb393ac093cec2d2980c26c297a69148c30f6639。
- App：项目 dist/All-in-One Toolbox.app（本地未签名构建）。
- 最终可执行文件 SHA-256：b1de0e7e7d1f4cd64fc56424437aa4d29ffc5c2050d22719640f6b35116e131a。
- 构建步骤：pnpm --dir src-ui run build；bash scripts/package_macos_app.sh。
- QA App：.runtime_data/round2-qa/All-in-One Round2 QA.app；Bundle ID com.allinone.desktop.round2qa。仅调整 Info.plist 身份和 LSEnvironment，可执行文件与交付包一致。
- 数据目录 .runtime_data/round2-qa-data，与日常数据库隔离；最初停用剪贴板、脚本、统计；不读取用户真实 API Key。标题采集关闭，统计只观察测试 App 自身。翻译测试文本为 Round2 integration QA only。
- [构建清单](round2-build-2026-10-25.json) 保留初始与最终构建映射；证据 01–06 来自前一构建，07–09 来自最终构建。两构建之间仅有本报告所述两处 GUI 文案及目录测试修正。

## 自动化验证
| 检查 | 结果 | 证据 |
| --- | --- | --- |
| cargo test --workspace --offline --all-targets | 166 通过，0 失败，2 ignored | [Rust 日志](evidence/round2-2026-10-25/rust-tests.log) |
| pnpm --dir src-ui run build | TypeScript 严格检查、Vite 构建通过；有体积提醒 | [构建日志](evidence/round2-2026-10-25/ui-build.log) |
| pnpm --dir src-ui test | 18 通过 | [前端日志](evidence/round2-2026-10-25/ui-tests.log) |
| cargo fmt --all -- --check | 通过 | 执行记录 |
| 最终 release --ui-smoke-test | PASS：原生窗口、开关、清理、恢复、快捷键注册 | [烟测日志](evidence/round2-2026-10-25/native-smoke.log) |

两个 ignored 是需要桌面会话的原生窗口枚举及原生活跃应用检查，严格断言未弱化。CLI 查询 CGSession 无会话时失败，实际 AppKit GUI 能取得真实会话并计时，二者分别记录。最后一次烟测在沙箱内无法访问 macOS 桌面服务，终止该测试进程后以桌面权限重跑最终可执行文件，结果 PASS。烟测只操作本程序自己的窗口和临时数据，不代表物理按键或完整截图交互已验收。

## 实际 GUI 用例与结果
使用 Computer Use 实际点击、输入、Esc 和重启，不通过 IPC 直接代替界面断言。专项原始计划：[截图](GUI_TEST_SCREENSHOT.md)、[统计](GUI_TEST_APP_USAGE.md)、[翻译](TRANSLATOR_GUI_TEST_PLAN.md)。

| ID | 步骤与预期 | 实际结果/证据 | 结论 |
| --- | --- | --- | --- |
| R2-01 | 启动，查看十槽位与状态 | 九模块已挂载，密码本禁用待开发，macOS 外部置顶明确不支持；[01](evidence/round2-2026-10-25/01-nine-module-catalog.png) | 通过 |
| R2-02 | 启用统计，测试 App 保持前台并刷新 | 真实 Bundle ID 和进程路径，最初 10 秒，最终持久化 52 秒；[02](evidence/round2-2026-10-25/02-native-usage-counting.png) | 本 App 采样通过 |
| R2-03 | 忽略本 App、重启后查看历史与空报表 | 忽略状态保留，报表排除被忽略应用，分类表仍保留 52 秒历史；空状态显示“暂无数据”；[08](evidence/round2-2026-10-25/08-final-usage-history-and-empty-text.png) | 通过 |
| R2-04 | 翻译页输入专用无敏感测试文本，点击翻译，未配置 Google Key | 明确显示未配置，不伪造翻译成功；[03](evidence/round2-2026-10-25/03-translator-missing-key.png) | 缺凭据路径通过 |
| R2-05 | 总览点击翻译“测试触发”，观察首次浮窗 | 首次正确显示无障碍权限不足；[04](evidence/round2-2026-10-25/04-popover-permission-guidance.png)、[09](evidence/round2-2026-10-25/09-final-popover-empty-result.png) | 权限/事件路径通过 |
| R2-06 | 浮窗 Esc | 返回主控台，窗口数恢复 1；最终空译文无错误忙碌提示 | 通过 |
| R2-07 | 截图页检查屏幕权限预检 | 开始截图禁用，明确指向系统权限设置，当前无冻结帧及贴图；[05](evidence/round2-2026-10-25/05-screenshot-permission-preflight.png) | 拒绝权限路径通过 |
| R2-08 | 分别关闭 01/05/07 | 对应导航与快捷键立即移除，其他模块仍可用；[06](evidence/round2-2026-10-25/06-three-modules-disabled.png) | 通过 |
| R2-09 | 点击退出，检查进程结束，重启同一 QA 数据目录 | 三开关保持关闭，页面正常；[07](evidence/round2-2026-10-25/07-restart-preserves-disabled.png) | 通过 |
| R2-10 | 最终构建重新开启三个模块，查看统计与翻译，再退出 | 入口/快捷键恢复，历史保留，浮窗 Esc 释放；最终退出后无 QA 进程 | 通过 |

## 尚未验收或实现
- QA App 未获屏幕录制与无障碍权限，未擅自扩大 OS 授权。因此截图拖选/标注/保存/真实剪贴板/贴图完整 GUI 闭环、多应用 AX 选区读取仍待授权实机验收。原生烟测不替代这些项目。
- 未使用真实翻译凭据，无官方 API 成功调用证据；隔离 HTTP/TLS/SSE 测试不等同服务账号验收。凭据只保存在会话内存，系统钥匙串未实现；不宣称内存已加密。
- 使用统计尚未对照实体锁屏/睡眠、多个外部应用、跨屏幕状态；音频 AFK 例外未实现。普通时间测试覆盖不等同全部实机场景。
- 实体键盘快捷键、Windows、多显示器及长时资源压力仍待测。测试触发按钮和快捷键注册烟测不冒充物理按键验收。
- 截图 M2 OCR/完整 L1-L2-L3/ONNX、M3 长截图/GIF，翻译 M2 鼠标钩子/Windows UIAutomation/更多服务源未完成。
- 共享剪贴板 RAII 租约未实现，翻译模拟复制与密码本秘密复制不能开放。密码本 M1 主密码闭环及 M2 真正生物识别封装均未实现。

## 后续顺序
1. 对 01/05/07 补权限和凭据后的实际闭环验收，另列 Windows/多屏/物理快捷键测试。
2. WorkBuddy 以已审查 M0 约束实现密码本 M1；先完成主密码和加密存储，独立审查后再挂载。
3. Codex 统一实现共享剪贴板租约，再开放依赖它的复制流程；01/07 后续阶段继续独立分支交付。

本轮本地合入 master，保留功能分支和原始提交；尚未推送远端。
