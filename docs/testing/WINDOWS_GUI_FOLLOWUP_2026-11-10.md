# Windows GUI 补充验收（2026-11-10）

本次使用 `0461dba` 合入的上一轮产物 `target/debug/all-in-one-desktop.exe`；代码与 2026-11-08 最终桌面构建相同。运行时指定隔离数据目录 `gui-round2-data`，没有操作真实密码本或系统认证。

## CodeBuddy 跨应用粘贴闭环

使用新建专用文件 `.runtime_data/windows-validation/paste-target-2026-11-10.txt`，初始为空。向系统剪贴板依次写入两个虚构文本 `Windows focus QA 2026-11-10 A`、`Windows focus QA 2026-11-10 B`，真实 watcher 收录后：

1. 点击 CodeBuddy 的文件编辑区，按 Ctrl+Shift+V。
2. 候选窗显示且搜索框获得焦点，初始第一项 B 高亮；应用日志报告原生激活成功及内部 WebView 交焦保留。
3. 按 Down，观察第二项 A 高亮。
4. 按 Enter，候选窗关闭，CodeBuddy 原编辑器出现 A 文本和未保存标记。
5. 按 Ctrl+S，未保存标记消失。随后仅从磁盘读取测试文件，内容严格等于 `Windows focus QA 2026-11-10 A`，29 bytes。没有通过代码写入目标文件来代替粘贴结果。

上轮因电脑工具检测到用户输入而未完成的最后存盘核对，本次已经补齐。CodeBuddy 属于 VS Code 类 IDE；WorkBuddy 本体、管理员权限目标及竞争焦点仍未覆盖。

证据：`final-2026-11-10-qa.stderr.log` 与上述实际目标文件。粘贴后日志仍有 Wry 窗口销毁时的 `PostMessage ... 0x80070578` 消息；本次候选窗退出、粘贴和保存均成功，未把这个清理日志隐去。

## GIF 动态帧和连续录制

同一隔离实例连续完成两次录制。第一段达到时长上限后自动显示非黑屏预览，并通过默认保存落盘：`screenshot_20261110_194950.gif`，502×302、254 帧、10694166 bytes。

第二段在录制期间对底层 CodeBuddy 测试文本执行 PageDown，随后用 Enter 停止、Ctrl+Shift+S 默认保存：`screenshot_20261110_195158.gif`，502×302、51 帧、2093074 bytes。独立 Pillow 解码全部帧，RGB 像素哈希有两个不同状态；直接观察解码首帧显示第 14–29 行，末帧显示第 46–61 行，确认真实页面变化进入了 GIF。首/中/末采样帧平均 RGB 亮度约 236–242，无黑帧。独立统计为 `final-2026-11-10-gif-analysis.json`。

这次还验证了自动结束后恢复预览、保存后重新创建遮罩、第二次正常停止与快捷保存。调试构建的实际采集帧率低于设置值，release 性能仍未验收；未把配置的 15 FPS 写成已达到的帧率。

## 启动与原生生命周期烟测

- `all-in-one-desktop.exe --smoke-test`：5 个阶段通过，退出码 0，日志 `final-2026-11-10-cli-smoke.log`。
- `all-in-one-desktop.exe --ui-smoke-test`：日志报告 `UI_SMOKE_RESULT: PASS (native windows, module switches, cleanup, restore, shortcuts)`。使用临时目录和本程序窗口，包含截图/贴图、剪贴板、便笺、屏幕注释窗口以及模块停用和恢复，不代表全功能硬件验收。

以上日志位于忽略目录 `.runtime_data/windows-validation/`。新增捕获审查及翻译气泡边界代码的验证另见各自报告；不能用本次旧产物的通过结果代替后续代码验证。

## 后续补丁与真实外框回归

集成 Antigravity `350765e`（Windows 翻译气泡工作区约束）和 Cursor `5c87fed`（GIF 排除登记有界失败、默认排除拒绝标识溢出）。合并后完整 Rust 工作区为 349 passed、0 failed、16 ignored，33 个测试程序；相关测试均为虚构数据，不一揽子运行硬件与认证专项。

为避免仅凭坐标乘除测试宣称建窗正确，Codex 将真实翻译窗口检查接入显式 `--ui-smoke-test`。首次原生结果为：工作区 1920×1032、缩放 1，气泡外框 376×269，位置 `(1560,772)`，超出右/下边界。原因是逻辑内容尺寸 360×260 未包含 Windows 原生尺寸边框。

建窗及重新显示时按 `outer_position` / `outer_size` 和当前显示器物理工作区二次校正，使用 `PhysicalPosition` 避免再乘 DPI。修复后实际外框保持 376×269，位置为 `(1544,763)`，右边为 1920、下边为 1032。`final-2026-11-10-native-bounds-corrected.stdout.log` 同时报告边界检查和完整原生生命周期烟测 PASS；首次失败日志也保留在相邻 `native-bounds` 文件中。

校正后再次完整执行工作区与最终桌面构建，最终记录为 `final-2026-11-10-workspace-corrected.log`、`final-2026-11-10-final-desktop-build.log` 和 `final-2026-11-10-final-ui.stdout.log`。本轮没有新增前端代码；前端 85 passed / TypeScript 与 Vite 通过的上一轮结果仍对应相同界面源码。

仍未覆盖：WorkBuddy 本体、管理员目标、真实混合 DPI 多屏、过大窗口需要缩小尺寸的交互，以及 release 安装与性能。不同屏幕逻辑坐标区间重叠的显示器选择歧义仍须跨层传递显示器标识或物理锚点解决。
