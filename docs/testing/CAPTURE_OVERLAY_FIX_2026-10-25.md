# 截图蒙层不显示修复与 GUI 验收（2026-10-25）

## 根因与复现
在已获得屏幕录制权限的正式 App 中点击「开始截图」，窗口数由 1 变为 2、后台会话进行中，但界面停在主控台。实际操作复现了用户报告。

宿主先隐藏创建 capture-overlay，前端同步写入冻结帧后却 await 两次 requestAnimationFrame，再发送 screenshot_overlay_ready 让宿主显示。隐藏 WKWebView 暂停动画帧回调，导致“等待显示前的动画帧”和“等待动画帧后的显示通知”互相等待。WebKit 对隐藏页面停止 rAF 的行为见 [官方说明](https://webkit.org/blog/8970/how-web-content-can-affect-power-usage/)。

此前原生烟测直接通过 IPC 调用 overlay_ready，绕过了真实前端初始化，因此未覆盖此问题；本轮用真实组件和实际 GUI 补足，不把后台会话创建当成蒙层可见。

## 修复
- 删除 CaptureOverlay.vue 初始化中等待两次动画帧的逻辑。冻结帧、遮罩像素仍先同步写入 canvas，await Vue nextTick 刷新提示 DOM 后，立即通知宿主显示窗口并聚焦。
- 保留正常交互中的 rAF 绘制调度；不提前显示未准备好的空白窗口，也不添加轮询计时器来掩盖初始化问题。
- 新增独立 capture-overlay.test.mjs，执行真实组件初始化脚本。模拟 rAF 永不派发，断言写入帧、绘制遮罩和刷新 DOM 后仍完成显示握手；也覆盖帧数据异常时呈现可取消的错误。
- 将新测试加入统一前端测试入口；Rust、权限适配器和签名策略本轮未改动。

## 自动化验证
- 修复前，新回归测试失败：hidden-window initialization must not wait for rAF；异常帧用例通过。
- 修复后，统一前端测试 26/26 通过（含两项新初始化回归）。[日志](evidence/overlay-2026-10-25/ui-tests.log)
- TypeScript 严格检查及 Vite 构建通过；仅保留已有包体积提醒。[日志](evidence/overlay-2026-10-25/ui-build.log)
- 正式打包并复用本机固定证书，签名要求保持一致。[打包日志](evidence/overlay-2026-10-25/package.log)
- git diff --check 通过。此次无 Rust 修改，未重复运行完整 Rust 测试。

## 实际 GUI 验收
使用 Computer Use 操作 dist/All-in-One Toolbox.app；没有通过 IPC 强制显示来代替用户入口。未改变用户模块开关、保存目录或剪贴板内容。

| 场景 | 实际结果 |
| --- | --- |
| 修复前点击开始截图 | 后台会话和隐藏窗口存在，未显示蒙层，复现成功 |
| 修复后点击开始截图 | 前台窗口变为 Capture Overlay，真实冻结桌面、暗色遮罩、智能框高亮和放大镜可见 |
| 再次启动后拖选 | 产生 351 × 161 像素选区，工具栏可见 |
| 点击矩形工具并在选区拖动 | 成功绘制，撤销按钮启用；颜色及线宽选项出现 |
| 点击撤销、重做 | 撤销/重做按钮按历史状态切换 |
| 按 Esc 退出 | 回到主控台，窗口数恢复 1，会话空闲且无冻结帧 |
| Cmd+Q 完全退出，确认进程结束后重新打开 | 权限保留，截图入口正常 |
| 重启后点击截图识字 | 同一蒙层正常显示，提示“选好区域后按 Enter 识别文字”，Esc 可退出 |

[重启后的真实蒙层 AX 记录](evidence/overlay-2026-10-25/restarted-ocr-ax.txt)；[取消后的 AX 记录](evidence/overlay-2026-10-25/after-cancel-ax.txt)；[取消后主控台截图](evidence/overlay-2026-10-25/after-cancel.png)。

可见选区/标注和重启后识字蒙层的完整桌面截图保存在本机 .runtime_data/overlay-qa/，因包含桌面背景和聊天列表未加入 Git；[构建清单](evidence/overlay-2026-10-25/build.json) 记录本地路径及 SHA-256。初始化测试使用专门的 2 × 2 测试帧，没有真实屏幕采集。

## 构建和边界
- macOS 27.0 arm64；基线 ae153ee 加本轮未提交改动，source_dirty=true。
- 最终可执行文件 SHA-256：8a73fb79c03297ebfb0d1821370e9024a85b6f9467b1c2adaa7ef0b4190d0e49。
- 沿用固定证书签名，未重新申请、重置或扩大系统权限。
- 本轮验收涵盖蒙层显示、框选、矩形工具、撤销/重做、取消与重启；没有把截图保存、贴图、剪贴板复制、OCR 识别结果、长截图/GIF、Windows 或多显示器标为本轮通过。
- 修复版 App 已保留在 dist，停留截图页供用户复核；本轮修改尚未提交或推送。
