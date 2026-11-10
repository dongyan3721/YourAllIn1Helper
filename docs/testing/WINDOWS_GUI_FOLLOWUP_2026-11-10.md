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
