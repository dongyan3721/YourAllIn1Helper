# Windows 后续分工（2026-11-10）

用户指定帮手为 Antigravity 与 Cursor。Codex 负责代码审查、集成、完整构建与 GUI 验收；两位帮手不启动应用或发送全局热键。

上一轮四项 GUI 修复及剪贴板焦点修复已于本轮合入本地 `master`，合并提交 `0461dba`；此前最终验证为 Rust 337 passed / 16 ignored、前端 85 passed、Windows 调试构建通过。结果与未验收范围见 [第二轮报告](testing/WINDOWS_GUI_ROUND2_2026-11-08.md)。没有推送本轮提交。

| 帮手 | 实际模型 | 分支与目录 | 任务 |
| --- | --- | --- | --- |
| Antigravity | Gemini 4 Argon Medium（已在界面选择） | `codex/windows-translator-bounds`；`D:\YourAllIn1Helper-worktrees\translator-bounds` | 补齐 Windows 翻译气泡展开后的工作区边界约束，覆盖 DPI、负坐标和过大窗口的纯逻辑测试 |
| Cursor | Grok 4.7 High（保留当前选择） | `codex/windows-capture-review`；`D:\YourAllIn1Helper-worktrees\capture-review` | 对照集成版审查旧 Qoder 四个未提交文件，只移植仍然必要的捕获生命周期修复，并补回归测试 |

两目录基线为 `f7fb8fc`，已配置 D 盘 Cargo target 与 jobs=1。不得修改主目录、升级依赖或改写锁文件，不 merge/push；完成后提交自己的代码与准确测试报告，由 Codex 审查合并。

审查期间旧 `D:\YourAllIn1Helper-worktrees\gif` 仅供只读对照。审查完成后，四个未提交源文件、原始 diff 与基线提交已保存到 `D:\YourAllIn1Helper-archive\windows-gif-followup-2026-11-10`，逐文件 SHA-256 与原件一致后才移除旧 worktree。必要的两项补充已由 Cursor 纳入集成，其他补充已有对应实现，详见审查报告。

## 交付与验证

- Antigravity / Gemini 4 Argon：`350765e`，补齐 Windows 工作区约束与五组纯逻辑测试。新会话执行；旧会话长期无输出后已停止。D 盘首次冷编译主动停止，未伪记为通过，由主目录统一验证。
- Cursor / Grok 4.7 High：最终提交 `5c87fed`，默认排除拒绝超出 u32 的句柄，GIF 在登记句柄变更或丢失时停止并清除半成品。自测 os-hal 2 passed、GIF 13 passed，无关 rustfmt 改动已还原。
- Codex 的真实窗口检查发现逻辑测试之外的系统边框：内容区 360×260，实际外框 376×269。新增建窗后物理外框校正与原生烟测，实际右下角位置从 `(1560,772)` 修正到 `(1544,763)`，完整外框落入 1920×1032 工作区。
- 主目录统一完整 Rust 回归为 349 passed / 0 failed / 16 ignored；校正后的最终日志为 `.runtime_data/windows-validation/final-2026-11-10-workspace-corrected.log`。桌面构建与原生窗口验证见 [GUI 补充验收](testing/WINDOWS_GUI_FOLLOWUP_2026-11-10.md)。
- 两个后续分支已经集成，其工作目录也已检查干净和提交可达后移除；保留分支引用和 Cursor 的测试日志归档。`git worktree list` 只剩主目录。Cursor 占用的 `capture-review` 空目录尚保留，不含源码、target 或 worktree 元数据。

混合 DPI 下不同显示器全局逻辑坐标重叠的既有歧义仍未解决；本机原生窗口验证为 100% DPI，不等于真实混合 DPI/多屏全部验收。长截图中途丢失排除登记时的错误终止策略仍是独立待办，不能把本次 GIF 有界失败测试当成该项通过。
