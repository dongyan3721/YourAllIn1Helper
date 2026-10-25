# Antigravity：功能 01 截图 M3（滚动长截图 + GIF 录制）

协调者：Cursor Agent / Claude Opus 5.5（接替 OpenAI Codex 统筹）。执行者模型保持 **Gemini 3.8 Flash**；界面模型不符时先校正，不擅自更换模型、不购买额度。

## 工作区

- 专用 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/antigravity-screenshot-m3`
- 分支：`codex/feature-screenshot-m3`；已合并本地 `master` `53240cc`（截图 M1、第二轮修正、剪贴板 RAII 租约、应用统计第三轮）。开工前执行 `git rev-parse HEAD` 并以该提交为起点。M2 在 `codex/feature-screenshot`（`65ad4e1`），尚未进入 master。
- 先执行 `pwd`、`git branch --show-current`、`git rev-parse HEAD` 并报告。只在本 worktree 写入与执行命令；不改主目录、其他成员目录（包括旧翻译 worktree），不 merge/push。使用本 worktree 独立 `target/` 与 `src-ui/node_modules`。

## 先读

`PROJECT_PLAN.md` 5.1（长截图、GIF 段落）、`docs/handoffs/screenshot-m1.md`（M1 架构：帧像素坐标、二进制 IPC、遮罩隐藏创建、`try_lock` 会话门、窗口层级）、`docs/development/REMAINING_FEATURES_PLAN_2026-10-25.md` 01 M3、`docs/MODULE_ARCHITECTURE.md`、`docs/testing/GUI_TEST_SCREENSHOT.md`。

## 并行协作边界（重要）

Cursor 同时在 `codex/feature-screenshot` 开发 M2（OCR、控件级选区），同样修改 `mod-screenshot`。为减少冲突：

- 新逻辑放新文件：`crates/mod-screenshot/src/scroll/`（帧采集、NCC 拼接）、`crates/mod-screenshot/src/gif/`（录制、缩放、编码）、对应 `tests/unit/` 与 `tests/` 新文件；前端新组件与 `src-ui/src/utils/screenshot/scroll*.ts`、`gif*.ts`。
- 对 `lib.rs`、`src-tauri/src/ipc/screenshot.rs`、`tauri_app.rs`、`CaptureOverlay.vue`、三语字典只做**追加式**最小修改：IPC 命令前缀 `screenshot_scroll_*`、`screenshot_gif_*`；字典只新增 `screenshot.scroll.*`、`screenshot.gif.*`。不重排、不重构已有代码。

## 本轮实际开发内容

### 1. 滚动长截图

- 入口：遮罩工具栏「长截图」按钮与快捷键 `screenshot:scroll`（默认 `Shift+F1`，冲突时回滚并报错，沿用 M1 处理）。
- 选区确定后：遮罩销毁或对选区鼠标穿透，显示选区边框与独立的小控制条（停止/取消/当前高度），控制条放在选区外，**不能被截入帧中**；层级参考 `src-tauri/src/screenshot_window.rs` 与 `ink_window.rs`，不能只靠 always_on_top。
- 后台线程以约 15 FPS 通过注入的 `ScreenCapturer` 采集选区（帧像素），用户手动滚动；可选自动滚动通过注入的 `InputSimulator::scroll_wheel_at`，缺辅助功能权限时如实提示只能手动滚动。
- 拼接：行特征 + 归一化互相关（NCC）求纵向位移；处理重复帧/无位移、固定顶部/底部导航栏剔除、低置信度跳过或停止并提示、反向滚动；长度上限（如 30000 帧像素）与内存上限，超限停止并说明。
- 结束后进入结果预览，复用 M1 的复制 / `create_new` 防覆盖保存 / 贴图。取消、Esc、停用模块、退出时停止线程并释放所有帧。

### 2. GIF 录制

- 入口：工具栏「GIF」与快捷键 `screenshot:gif`（默认 `SecondaryMod+F1`）。
- 选择分辨率（原画 / 1080p / 720p / 480p / 自定义宽高，保持比例）与帧率（10/15/24/30 FPS）；录制时同样显示选区外控制条（停止/取消/时长/帧数），控制条不进入画面。
- 用 `image` 缩放，**编码器使用 MIT/Apache 许可的 `gif` + `color_quant`（或 `image` 内置 GIF 编码）**。`gifski` 为 AGPL-3.0，与本项目 MIT 许可冲突，未经协调者确认不得引入。
- 流式编码，限制最长时长/帧数与内存；保存为真实可解码 GIF（`create_new` 防覆盖）。取消删除半成品；停用模块等待线程结束。
- 事件：`screenshot://scroll-frame-stitched`、`screenshot://gif-recording-stats`（若新窗口需监听，按最小权限新增 capability，不能依赖吞掉的监听错误）。

## 约束

- 业务、配置、测试分文件，中文注释；只依赖注入的 HAL trait，不直接依赖 `NativeOsHal`/Tauri/objc2；生产禁止 Mock 回退。
- 重任务不阻塞 AppKit 主线程；后台任务可取消且在 `on_shutdown` 中等待结束。
- 允许从 crates.io 官方源新增 `gif`、`color_quant` 等依赖，交接中写明版本与用途。
- 测试只用自制合成图像与临时目录，不截取用户真实屏幕内容作为测试数据。

## 验收与交接

- 单测：合成长页面按已知位移切帧 → 拼接结果逐像素一致；固定头尾；重复帧；无位移；低置信度；长度上限；取消释放；GIF 编码后用 `gif` 解码验证帧数/尺寸/帧间隔；分辨率换算；停用等待线程。
- `cargo fmt --all -- --check`、`cargo test -p mod-screenshot --offline`、`cargo test --workspace --offline --all-targets`、`pnpm --dir src-ui run build`、`pnpm --dir src-ui test`，如实记录。
- GUI 用例追加到 `docs/testing/GUI_TEST_SCREENSHOT.md`（新增 SHOT-M3-xx），未实际操作标「未执行」（协调者负责 GUI 验收）。
- 分批 commit，正文写 `Agent: Antigravity` 与 `Model: Gemini 3.8 Flash`。交接 `docs/handoffs/screenshot-m3.md`：提交、接口/配置、公共文件清单、真实测试结果、未执行项与平台限制。

先报告实际路径、分支、模型与步骤，然后立即编码，不只输出方案。阻塞时报告具体原因，不伪报进度。
