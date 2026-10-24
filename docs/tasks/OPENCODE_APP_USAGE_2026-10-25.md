# OpenCode：功能 05 使用时长统计

模型保持 DeepSeek V4.1 Flash 1017（`deepseek/deepseek-flash`）。用户此前已明确授权 OpenCode `--auto`，本次沿用；不修改全局权限/代理，不购买额度，不更换模型。

专用 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/opencode-app-usage`；分支 `codex/feature-app-usage`；基线 `4689c49`。先核验 pwd、分支和 HEAD。所有命令与写入仅限本 worktree，不修改主目录或旧脚本分支，不 merge/push。

先读 `PROJECT_PLAN.md` 5.5、`docs/MODULE_ARCHITECTURE.md`、`docs/development/REMAINING_FEATURES_PLAN_2026-10-25.md`。实际开发该计划 05 的 M1+M2，不停留于方案：

- 创建 `mod-app-usage`，注入真实 `ActiveAppWatcher`，平台实现独立文件。macOS 使用稳定 Bundle ID、真实空闲/锁屏/息屏状态；不能把进程名拼接成真实 ID，不用 Unsupported 返回假布尔值。
- 单调时钟计时、焦点切换/每 60 秒刷写、跨小时/跨日切段、休眠跳变、停用刷写/停止、重启不重复计时。
- `/usage` 仪表盘：每日、小时、分类；应用分类/忽略名单与配置持久化；zh/en/ja、主程序即时开关、禁用 IPC、保留原六模块数据。
- 音频活动的 AFK 例外需要真实能力；未实现时明确列限制，不假装完整满足原说明书。
- 不采集按键内容；测试只用隔离数据，不开启日常应用的后台统计。窗口标题保留可配置。
- 只补充自己需要的公共接线，列清共享文件；不重写整个宿主、不覆盖其他字典，不触碰截图/翻译/密码本目录。保留工具栏层级和事件权限修复。
- 配置、业务与测试分文件，中文注释。准备可注入时间源的有意义测试及 GUI 用例；GUI 未实际操作就标未执行，不能以 IPC 替代。
- 运行 Rust 格式、模块/工作区离线测试、前端严格 build 与 test。依赖缺失如实记录，可获取官方依赖但不更改系统配置；用本 worktree 依赖目录与独立 target，避免与其他 agent 并发争用。
- 分批 commit，正文记录实际智能体/模型。交接 `docs/handoffs/app-usage.md`：提交、接口/迁移/配置、公共文件清单、真实测试和未执行项。

先报告实际路径、分支、模型与步骤，然后立即编码。阻塞时报告具体原因，不伪报进度。
