# Windows GIF 黑屏与实时捕获遮罩排除（2026-11-08）

基线 `0cdada8`，分支 `codex/windows-gif-capture`，独立工作目录 `D:\YourAllIn1Helper-worktrees\gif`。本轮只改 os-hal Windows 捕获、截图遮罩的 Windows 注册、GIF recorder 与测试；未修改 `CaptureOverlay.vue`，未启动桌面应用、未发送全局快捷键、未做 GUI 自动化。

## 根因

1. **Windows 从未登记遮罩句柄。** `present_capture_overlay` 只在 macOS 分支把遮罩窗口号写入 `ScreenshotModule::overlay_window_id`；Windows 分支只显示窗口。因此 GIF 启动时 `exclude_window_id` 恒为 0。
2. **`capture_region_below` 在 Windows 直接退回普通抓取。** 旧实现收到 0 就调用 `capture_region`，收到非 0 也忽略窗口号。遮罩仍留在画面里，且 recorder 因 `exclude_window_id == 0` 不再合成标注墨迹。
3. **xcap 0.9.8 默认的 GDI 路径无法正确合成透明 WebView2 遮罩。** 项目未启用 `wgc` feature，`xcap` 在 Windows 用 `GetWindowDC(GetDesktopWindow()) + BitBlt(SRCCOPY)`；透明遮罩窗口的 WebView2/DComp 表面被抓成黑色。于是 GIF 每帧是黑面（或冻结遮罩），而不是选区下方真实画面。
4. **`u32` 无法承载 Win64 HWND。** 不能把 64 位 HWND 截断后当作 macOS 的 `CGWindowID` 复用。

高 DPI 不是本次黑帧根因：Windows 的 `DisplayMonitorInfo` 与 `SelectionBox` 使用物理像素，`MonitorGeometry::frame_selection_to_desktop` 在 Windows 保持 1:1，冻结帧、选区和 xcap 区域抓取同一坐标空间。已有 `windows_high_dpi_desktop_units_are_physical_pixels` 几何测试本轮通过。

## 实现

- `ScreenCapturer` 新增默认方法：
  - `set_capture_exclusion(Option<WindowHandleId>)`：登记/撤销原生捕获排除；
  - `active_capture_exclusion()`：返回当前登记的 64 位句柄；
  - `capture_region_excluding(bounds, Option<WindowHandleId>)`：按句柄抓取。默认实现对可放进 `u32` 的标识转交 `capture_region_below`，旧适配器不受影响。
- `NativeOsHal` 保存 `Arc<Mutex<Option<WindowHandleId>>>`，Windows 调用 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`：窗口从 BitBlt/WGC/桌面复制等抓屏结果中完全消失，露出底层真实内容，但屏幕显示与交互不变。撤销时恢复 `WDA_NONE`；GIF 每帧抓取前再重申一次，避免被其他代码重置。`WDA_EXCLUDEFROMCAPTURE` 需要 Windows 10 2004+；更早系统会退化为 `WDA_MONITOR`（抓屏黑块），属于系统能力边界。
- `ScreenshotModule` 新增 `set_overlay_native_handle` / `clear_overlay_native_handle`；会话结束 `finish_session_locked` 先撤销排除再销毁遮罩，避免系统复用句柄后误排除其他窗口。
- `src-tauri/src/screenshot_window.rs`：Windows 显示遮罩后读取真实顶层 HWND，按 `usize -> u64` 完整登记；读取/设置失败只记录警告，不阻断截图流程。
- GIF recorder：macOS 继续使用显式 `CGWindowID`；Windows 在显式标识为 0 时读取已登记的 64 位 HWND。抓取改为 `capture_region_excluding`，只要排除生效就把前端标注 PNG 重新合成进每一帧。
- 长截图无需改代码即可复用：`ScrollCaptureSession` 仍以 0 调用 `capture_region_below`，Windows 平台实现会自动使用已登记的遮罩 HWND；Antigravity 不需要维护第二套排除实现。
- 未升级依赖、未改 `Cargo.lock`、未新增 Windows feature。macOS 的 `CGWindowListCreateImage(OptionOnScreenBelowWindow)` 行为保留。

## 测试记录

所有命令均在 `D:\YourAllIn1Helper-worktrees\gif` 执行，`CARGO_TARGET_DIR=D:\YourAllIn1Helper-worktrees\gif\target`，`.cargo/config.toml` 已限制 `jobs = 1`。

| 项目 | 命令 | 结果 |
| --- | --- | --- |
| GIF/截图模块单测 | `cargo test -p mod-screenshot --lib` | **42 passed, 0 failed**；含新增黑帧/排除/墨迹合成用例 |
| os-hal 单测 | `cargo test -p os-hal --lib` | **36 passed, 0 failed, 4 ignored**（原有 ignored 为真实剪贴板/桌面会话专项） |
| Tauri 宿主编译 | `cargo check -p all-in-one-host --lib` | **通过，exit 0**；`Finished dev profile in 12m 25s`，仅 2 条既有 warning（未使用导入等，不在本轮改动文件内） |

日志位于 D 盘 worktree 的 `target/`：`test-mod-screenshot-final.log`、`test-os-hal-final.log`、`check-host.log`。

新增测试：

- `test_gif_recording_blends_ink_when_capture_excludes_overlay`：抓屏端返回黑帧并收到排除标识，解码后的 GIF 首帧必须是重新合成的红色墨迹，防止“排除后只剩黑帧”。
- `test_gif_recording_keeps_capture_when_overlay_is_not_excluded`：未登记排除时不重复叠墨迹，保留抓到的蓝色画面。
- `test_gif_encoder_preserves_non_black_content`：纯白 RGBA 输入编码后解码仍为亮色，防止编码/量化路径产生黑帧。
- `explicit_below_window_id_takes_precedence`：显式 `u32` 遮罩号优先，0 时回退到已登记 64 位句柄。
- `windows_hwnd_keeps_all_64_bits`：64 位 HWND 全位保留、0 句柄拒绝。
- `clearing_an_unregistered_exclusion_is_safe`：未登记时撤销排除安全无副作用。

## 未测项与边界

- **未做真实桌面验收**：按分工由 Codex 集中验收。以下项目本轮未运行，不能记为通过：真实 GIF 在 Win11 上的录制与动画播放、遮罩透明洞口是否露出真实动态画面、WDA 排除在 BitBlt 下的实际效果、目标应用（浏览器/Office/VS Code）中的 GIF 粘贴动画、混合 DPI/负坐标副屏、旋转屏。
- **未创建原生窗口做 WDA 集成测试**：`SetWindowDisplayAffinity` 需要当前进程拥有的顶层窗口；按“不启动桌面应用、不做 GUI 操作”约束未创建测试窗口。`windows_hwnd_keeps_all_64_bits` 只验证句柄未被截断。
- **未启用 xcap `wgc`**：保持依赖与抓屏路径不变。若实机仍观察到 GPU 合成窗口黑面，可评估 WGC 或 DXGI Desktop Duplication，但那会改变其他截图路径，需单独验证。
- 旧于 Windows 10 2004 的系统不支持“排除后露出底层”，会表现为黑块；这是系统限制，不宣称修复。
- macOS 仅做条件编译保留与单元测试，未在 macOS 实机重跑；`capture_below_window` 代码未改动。

## 共享接口（交 Codex 集成）

以下为最小增量改动，供集成与交叉审查：

- `crates/os-hal/src/traits.rs`：新增三个 `ScreenCapturer` 默认方法（见上），不要求其他实现立即改动。
- `crates/mod-screenshot/src/lib.rs`：
  - 新增 `ScreenshotModule::set_overlay_native_handle(WindowHandleId) -> CoreResult<()>`；
  - 新增 `ScreenshotModule::clear_overlay_native_handle()`；
  - `finish_session_locked` 增加一次清理调用。
- `src-tauri/src/screenshot_window.rs`：Windows 遮罩显示后登记 HWND。
- 现有 `overlay_window_id: AtomicU32` 与 `set_overlay_window_id` 保留给 macOS，未改共享标识类型；Windows 不经过该字段，避免 HWND 截断。
- `CaptureOverlay.vue` 无需改动：现有 `live-hole` 透明洞口、`overlay_png_base64` 标注层与 GIF 启停接口已足够；本轮没有新增前端接口。

## 手动复测建议（Codex）

1. Win11 下框选一块含动画/视频的区域，开始 GIF 录制，确认预览与保存文件不是黑屏，选区下方内容真实变化。
2. 在选区上画一笔后再录制，确认笔迹出现在每帧且没有叠加两次。
3. 录制时观察遮罩边缘/工具栏是否出现在 GIF 中；应完全不可见。
4. 长截图使用同一遮罩，确认蓝框与遮罩不进入拼接图（Antigravity 验收）。
5. 结束截图后再次普通截图，确认遮罩窗口仍正常显示，没有因残留 WDA 被系统排除。

参考：[SetWindowDisplayAffinity](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)（`WDA_EXCLUDEFROMCAPTURE`，Windows 10 2004+）。
