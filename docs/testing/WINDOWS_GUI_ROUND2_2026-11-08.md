# Windows GUI 第二轮最终集成（2026-11-08）

基线 `0cdada8`。本轮代码在 `codex/windows-gui-fixes` 集成，四项用户确认问题已修复；额外集成剪贴板候选窗焦点修复。原生交互使用隔离数据目录与虚构文本，不操作真实密码本认证。

## 四项修复及证据

| 项目 | 最终处理 | 实测范围 |
| --- | --- | --- |
| GIF 黑屏 | Qoder `ac88785` 提供 Win64 HWND 登记与 WDA 排除；Codex `5da9e23` 把排除限于实时采集、检查启动条件、清理失败半成品，并在停止/取消/自动结束时恢复预览 | 真实记事本文本选区录制与应用内预览正常；保存 GIF 为 556×354、54 帧。首/中/末帧均可独立解码，平均亮度约 227，不是黑帧 |
| 长截图蓝框 | Antigravity `e2c9829` 外绘边框、隐藏实时采集的手柄与标签；复用 Windows 原生排除，不裁掉真实选区边缘 | 在记事本手动连续向下滚动，从 556×481 拼接为 556×737，预览行号连续；PNG 边缘无 `#0a84ff`，无贯穿宽度的蓝线 |
| 保存路径 | CodeBuddy `02aab7a` 后端，Codex `fd66f0c` 完成各入口；`90ee1d3` 修正中文路径请求头编码 | 原生选择器、取消、默认快捷保存、中文与空格路径落盘均通过。详见 [保存报告](WINDOWS_GUI_SAVE_2026-11-08.md) |
| Windows 字体 | `0b96509` 统一 Segoe UI / Microsoft YaHei UI 无衬线回退；普通便笺、剪贴板预览不用强制等宽字体，代码字体保留等宽并补中文回退；截图预览和画布标注也统一 | 便笺中英文混排、剪贴板表格及候选卡片已观察，无宋体衬线回退；代码和数学排版仍保留自身用途 |

长图中能看到电脑工具自身的局部指针高亮；没有把它编辑掉冒充干净的原生结果。选区边框污染检查针对图像边缘及贯穿横线，且真实选区尺寸保持不变。

## 额外的剪贴板焦点修复

Zed Agent / Sonnet 5.5 提供 `7ac74df` 与报告 `bc07909`：验证 Windows 前台 HWND、把焦点交给 WebView2、窗口级键盘监听、IME 保护和过期粘贴目标清理。Codex `cec7d0f` 继续修复真实 GUI 中出现的生命周期问题：

- Windows 热键创建候选窗和有界激活等待放到后台，让 UI 主线程继续派发激活消息；创建时交由显式前台策略处理焦点。
- WebView2 内部交焦可能产生 `Focused(false)`。若 `GetForegroundWindow` 仍是候选窗，不能当作用户离开而立即销毁；真正切走时仍按瞬态窗口策略关闭。
- 调试日志证实 `Clipboard popup activation: Ok(())` 及内部交焦保留路径。

GUI 已验证：直接呼出持续显示、方向键从虚构 item B 移到 item A；在 CodeBuddy 的专用 `paste-target-final.txt` 编辑器按 Ctrl+Shift+V，候选窗保持打开，Down 改变高亮，Enter 关闭候选窗并返回原编辑器，出现未保存编辑状态。**最后 Ctrl+S 两次被电脑工具的用户输入检测中断，未核对落盘文本，不能将这一环写成完整跨应用存盘验收通过。** WorkBuddy 本身与管理员权限 IDE 仍需复测。

## 统一验证

- `cargo test --workspace --locked --all-targets -j2`：最后焦点生命周期改动后完整重跑，337 passed、0 failed、16 ignored；33 个测试程序。
- `pnpm --dir src-ui test`：85 passed、0 failed。
- `pnpm --dir src-ui run build`：TypeScript 与 Vite 通过。
- `pnpm build:desktop`：Windows 调试桌面构建通过。完整 Rust 测试会覆盖调试 exe，故最终启动前再执行桌面构建，避免运行指向开发服务器的产物。
- 最后的焦点专项纯逻辑测试：6 passed；真实焦点状态不由这些纯逻辑测试替代。

日志和 GUI 产物保存在 `.runtime_data/windows-validation/`；硬件、认证及覆盖真实剪贴板的 ignored 用例没有一揽子运行。macOS 编译、混合 DPI/副屏、release 安装与性能仍不在本轮已验收范围。

## 分支和磁盘

- 初始四个 C 盘旧 worktree 已检查提交属于原 master、无未提交源码，保留分支引用后清理，释放约 11.8 GiB。旧 secure-services 运行数据另备份到 D 盘 archive。
- 本轮三个已合入且干净的 D 盘 worktree（scroll/save/focus）也已移除；提交和交付文档在集成分支中保留。
- `D:\YourAllIn1Helper-worktrees\gif` 存在 Qoder 后续未提交的四个源文件修改，**没有删除或重置**。其中启动拒绝/旧句柄恢复等问题已由集成版本以实时采集预检和完整清理处理；该目录保留供后续对照，不能说所有 worktree 都干净。
- 本轮构建后 C 盘约余 24 GiB；新增并行构建均位于 D 盘。未推送本轮提交。

剩余 Windows 范围以 [适配复查](WINDOWS_ADAPTATION_AUDIT_2026-11-08.md) 及 [待测清单](WINDOWS_HANDOFF_CHECKLIST_2026-11-07.md) 为准。翻译气泡展开后的屏幕边界约束仍是独立缺口，没有把已有 Windows OCR、划词提取或使用时长实现误列为未迁移。
