# Windows 后续分工（2026-11-10）

用户指定帮手为 Antigravity 与 Cursor。Codex 负责代码审查、集成、完整构建与 GUI 验收；两位帮手不启动应用或发送全局热键。

上一轮四项 GUI 修复及剪贴板焦点修复已于本轮合入本地 `master`，合并提交 `0461dba`；此前最终验证为 Rust 337 passed / 16 ignored、前端 85 passed、Windows 调试构建通过。结果与未验收范围见 [第二轮报告](testing/WINDOWS_GUI_ROUND2_2026-11-08.md)。没有推送本轮提交。

| 帮手 | 实际模型 | 分支与目录 | 任务 |
| --- | --- | --- | --- |
| Antigravity | Gemini 4 Argon Medium（已在界面选择） | `codex/windows-translator-bounds`；`D:\YourAllIn1Helper-worktrees\translator-bounds` | 补齐 Windows 翻译气泡展开后的工作区边界约束，覆盖 DPI、负坐标和过大窗口的纯逻辑测试 |
| Cursor | Grok 4.7 High（保留当前选择） | `codex/windows-capture-review`；`D:\YourAllIn1Helper-worktrees\capture-review` | 对照集成版审查旧 Qoder 四个未提交文件，只移植仍然必要的捕获生命周期修复，并补回归测试 |

两目录基线为 `f7fb8fc`，已配置 D 盘 Cargo target 与 jobs=1。不得修改主目录、升级依赖或改写锁文件，不 merge/push；完成后提交自己的代码与准确测试报告，由 Codex 审查合并。

旧 `D:\YourAllIn1Helper-worktrees\gif` 仅供只读对照，四个未提交源文件保持原状，不删除、不重置。已清理的旧 worktree 不重建。恢复任务时 C 盘约余 32 GiB，D 盘约余 673 GiB。
