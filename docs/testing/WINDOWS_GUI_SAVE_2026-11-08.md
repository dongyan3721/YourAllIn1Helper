# Windows 截图另存为与默认保存（2026-11-08）

CodeBuddy 提供后端提交 `02aab7a`；Codex 接管剩余界面、集成与实测（`fd66f0c`、`90ee1d3`）。CodeBuddy 的宿主测试任务中途停止，不能记为其已完成整套交付；以下统一回归由 Codex 在集成目录执行。

## 最终行为

- 普通选区、长图预览、贴图与 GIF 确认提供「另存为…」及「默认保存」。选区和贴图中 Ctrl+S 选择位置，Ctrl+Shift+S 使用配置目录；GIF 预览支持同样的快捷键。
- Windows 使用后台 STA 线程运行 `IFileSaveDialog`，对话框属主为当前截图窗口；UTF-16 路径不进入 shell。过滤器约束 PNG/GIF，系统负责覆盖确认；取消返回空路径，前端不导出、不退回默认保存。
- 默认目录保存沿用时间戳和独占创建，不覆盖现有文件。指定路径先写同目录临时文件并刷新，再原子替换目标；失败不截断原文件。
- GIF 确认写入失败时保留待确认会话和临时文件，允许重新选择路径；重复录制的预览并发标志也会复位。

## 实测发现并修正的中文路径问题

普通截图的标注 PNG 使用二进制 IPC，导出参数位于请求头。直接把含中文路径的 JSON 放入 `Headers` 会违反 ByteString 约束；路径选择成功后仍然无法导出。最终使用 ASCII JSON Unicode 转义，后端继续用原来的 JSON 解析，中文、emoji 与反斜杠均原样恢复，无需修改协议或新增依赖。

回归测试同时验证原始中文 JSON 会被 `Headers` 拒绝，以及实际编码后的请求头可以构造并无损解析，避免只测文件写入而漏掉传输层。

## 验证

1. 最终前端：85 passed，TypeScript/Vite 通过。
2. Rust 完整工作区命令与结果见 [最终集成报告](WINDOWS_GUI_ROUND2_2026-11-08.md)。覆盖指定路径、默认路径、扩展名、GIF 真实编码及失败后重试；对话框参数、单次路径选择和取消均有独立纯逻辑测试。
3. Windows GUI：Ctrl+S 显示系统保存选择器；取消后选区仍可继续操作，截图目录没有新增文件。
4. Windows GUI：Ctrl+Shift+S 不打开选择器，产物 `gui-round2-data/screenshots/screenshot_20261108_195400.png` 成功落盘。
5. 修正传输后，GUI 另存为 `截图 保存测试/截图 中文 01.png` 成功；独立 Pillow 解码确认 PNG、491×276、29375 bytes。

证据均在主目录忽略路径 `.runtime_data/windows-validation/`，包含上述产物和 `gui-round2-final-ui-tests.log`、`gui-round2-final-ui-build.log`、`gui-round2-final-workspace-tests.log`、`gui-round2-final-desktop-build.log`。

## 未验收边界

没有点击真实覆盖确认、没有单独走完 GIF/贴图/长图每一种原生另存对话框，也没有在 macOS 运行新 NSSavePanel 路径。相应核心保存分支通过自动化测试；这些不能写成全平台 GUI 全覆盖。
