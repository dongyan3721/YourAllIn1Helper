# Windows beta GIF 录制反馈修复（2026-11-12）

基线 `4a61afc`，修复分支 `codex/fix-windows-gif-capture`。用户反馈：网易 UU 远程虚拟屏中开始 GIF 后退回截图界面，正常桌面录制时出现黑底、静止预览和静止 GIF。

## 改动

- 原路径保留全屏 WebView2/DComp 冻结遮罩，只清空 Canvas/CSS 并设置 `WDA_EXCLUDEFROMCAPTURE`。旧报告明确未验证真实动态桌面；mock 编码测试不能证明采集到了桌面变化。
- Windows 实时采集改为在首帧前使用 `SetWindowRgn` 从遮罩的原生窗口区域中减去精确的录制矩形。录制区域不再有遮罩窗口内容，底层画面和鼠标交互直接露出；不再设置 WDA 内容保护标记，不让远程软件把整个遮罩排除/显示为黑块。
- GIF 与长截图共用准备接口；逐帧核对正在录制的窗口句柄和选区，未准备好时明确失败，避免拍到冻结遮罩。停止、取消、切换会话均恢复完整窗口区域。
- 坐标使用实际 HWND 全局物理像素，包含负坐标副屏校验。macOS 保持原有窗口以下采集路径。
- 停止后编码超过同步等待时限时，前端继续轮询，不再退回静态截图工具。防止重复点击启动；录制和预览状态使用普通光标。
- 录制控制条改为横向排列并重新测量定位，计时/帧数使用清晰的浅色文字。
- 全屏选区会裁掉整个遮罩，录制中再次触发截图/GIF 快捷键或托盘截图入口改为结束录制并保留原会话，避免新建截图丢掉录制；全屏可通过此入口停止。

## 验证

| 检查 | 结果 |
| --- | --- |
| 截图/GIF 模块单元测试 | 54 通过 |
| OS HAL 单元测试 | 42 通过，4 个既有专项忽略 |
| 前端单元测试 | 97 通过，含慢速 GIF 收尾状态回归 |
| 前端 TypeScript | 通过 |
| GIF/长截图生命周期集成测试 | 3 通过，截图快捷键结束 GIF 且原会话保持不变 |
| 真实 WebView2 动画录制 | 28 帧：红 15、蓝 13；第二次 30 帧：红 14、蓝 16 |
| 原生窗口独立检查 | 选区不属于遮罩的窗口区域；DisplayAffinity 为 WDA_NONE |
| GUI 选区→开始→停止→预览→默认路径保存 | 完成；预览实际出现红/蓝两种画面 |
| GUI 保存文件独立解码 | 237 帧，342×202；红 118、蓝 119，278046 字节 |
| 优化版内嵌前端，区域录制 | 30 帧：红 14、蓝 16，截图动作结束录制并保留预览 |
| 优化版内嵌前端，全屏录制（480p） | 19 帧：红 12、蓝 7，整个遮罩被裁掉后仍可从截图动作结束录制 |
| 录制控制条修正后 GUI | 横向、计时和帧数可见；点击停止后进入动态预览 |
| 最终 MSI 对应 EXE 再测 | 区域 30 帧、全屏 13 帧，均含红/蓝动态；已核对 EXE 内嵌最终 CaptureOverlay 资源名 |
| MSI 更新签名独立验证 | 原包通过、签名版本为 0.1.0-beta、修改一个字节后验证拒绝 |

GUI 和原生烟测均使用隔离的临时应用数据、自产红蓝动画窗口；未改动正常安装版的数据、凭据或密码本。保存的 GUI 测试动画另复制到 `.runtime_data/gif-gui-saved.gif`，统计记录为 `.runtime_data/gif-gui-result.json`。

可复跑专项烟测（debug EXE 需要 Vite 开发服务；release EXE 使用内嵌前端）：

```powershell
$env:ALL_IN_ONE_GIF_SMOKE_ONLY = '1'
./target/release/all-in-one-desktop.exe --ui-smoke-test
Remove-Item Env:ALL_IN_ONE_GIF_SMOKE_ONLY
```

另设 `ALL_IN_ONE_GIF_SMOKE_FULLSCREEN=1` 可验证全屏选区（输出 `.runtime_data/gif-native-fullscreen-qa.gif`）。

`ALL_IN_ONE_GIF_GUI_QA=1` 配合 `--keep-open` 会在测试成功后保留自产动画窗口并打开截图面板，用于人工 GUI 验收。

## 远程验收边界

本机检测到网易 UU 的 `GameViewer Virtual Display Adapter` 及运行中的 GameViewer 服务。已去掉对 WDA 的依赖并验证本机动态采集；目前没有建立 UU 虚拟屏远程会话，不能将该特定环境记为已通过。应在 UU 客户端连接虚拟屏后复测录制控件可见性、区域动态画面、停止预览与文件动画。

本地验证安装包在 `dist/gif-fix-2026-11-12`，显示版本保留 0.1.0-beta；未替换 GitHub 已发布的附件和更新 feed，也未覆盖本机正常运行的安装版。正常退出旧版后可用该包覆盖安装进行 UU 验收。

既有 Tauri mock updater 集成测试在本环境启动时返回 `0xc0000139 / STATUS_ENTRYPOINT_NOT_FOUND`（重新编译后仍如此），本轮没有将其记为通过。安装包验证改用 `scripts/verify_windows_updater.rs`，直接调用同一 `minisign-verify` 库并独立校验真实产物、公钥、签名版本和篡改拒绝，不加载 Tauri/WebView2，也不执行安装。

两份 `--keep-open` GUI QA 临时目录的递归清理被工具自动审批拒绝（仅返回 `blocked by policy`），已停止重试。它们位于 `%TEMP%\all-in-one-ui-smoke-19668` 与 `%TEMP%\all-in-one-ui-smoke-23504`；测试进程和临时开发服务已退出。

原生窗口裁剪依据：[Microsoft SetWindowRgn](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowrgn)。
