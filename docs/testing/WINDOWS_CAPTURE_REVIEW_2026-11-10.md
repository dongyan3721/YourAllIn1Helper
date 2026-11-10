# Windows 捕获遗留补丁审查（2026-11-10）

基线 `f7fb8fc`，分支 `codex/windows-capture-review`，只在 `D:\YourAllIn1Helper-worktrees\capture-review` 修改。对照对象是 `D:\YourAllIn1Helper-worktrees\gif` 里尚未提交的 4 个文件，只读 `git diff`（`recorder.rs`、`lib.rs`、`platform/screen.rs`、`traits.rs`，+114 / -22）。该目录没有被修改、提交或清理。

Round 2 已修复 GIF 黑屏、长截图蓝框、保存和字体，见 [第二轮记录](WINDOWS_GUI_ROUND2_2026-11-08.md)。本轮不重复搬运启动预检、结束恢复和失败半成品清理。未启动桌面、全局热键或 GUI 自动化，未改锁文件或依赖，未处理密码本与认证。

## 逐项取舍

| 旧补丁 | 当前集成 | 本轮 |
| --- | --- | --- |
| GIF 循环每帧重读登记句柄；变成 `None` 时在循环内写 `status.error` 并 `break`。句柄换成新值时继续用新 HWND 录 | 进入循环前把句柄快照一次。抓取失败、零帧和取消已经走 `recording_error` / 取消分支，删除临时文件且不留下 `pending_path` | **移植为有界失败。** 显式非 0 窗口号整段固定。登记句柄变更或丢失时写入已有的 `recording_error` 并立刻 `break`，因此已编码的帧也会被删掉，不能当成可预览 GIF。不改用新句柄继续录 |
| `ScreenshotModule` 记住遮罩排除失败文案，`start_gif` / `start_scroll` 前拒绝 | `prepare_live_capture` 在 Windows 上要求已有登记，失败则不开录；会话创建或控制窗失败时调用 `finish_live_capture` | **不移植。** 启动拒绝已经由预检负责 |
| 登记时立刻 `SetWindowDisplayAffinity`；同一把锁里先恢复旧窗口；新窗口设置失败则把登记清成 `None`；附带 `exclusion_transition` | 登记只校验并保存句柄。WDA 在 `prepare_live_capture` 启用，`finish_live_capture` 恢复并把失败返回调用方。替换时已在同一把锁里恢复不同的旧句柄；新句柄校验失败不改已有登记 | **不移植。** 那套立刻设置 WDA 的失败清理和现在的预检/结束模型不是同一条生命周期。清空登记会在新句柄无效时丢掉仍有效的排除 |
| 默认 `capture_region_excluding` 对放不进 `u32` 的标识返回错误，不再静默 `capture_region` | `NativeOsHal` 已覆盖该方法：Windows 用完整 HWND 重申 WDA，macOS 对超范围标识返回「CGWindowID 超出 32 位」。默认方法仍会丢掉高位并普通抓屏 | **移植。** 只改 trait 默认方法。能放进 `u32` 的非零标识仍交给 `capture_region_below`；`0` 和 `None` 仍是普通抓取。原生 Windows / macOS 覆盖实现未改 |

句柄更换不继续录，是因为这段 GIF 开始时由 `prepare_live_capture` 武装的是旧窗口。沿用旧快照会把 WDA 重申到可能已失效的 HWND；改跟新句柄则会把两段窗口拼进同一个文件。两种都会在排除已不再是本段录制的那一个时仍报成功。

长截图不在这 4 个文件的有效缺口里。Windows 长截图以窗口号 0 调用 `capture_region_below`，原生实现每次都会读当前登记。登记被清空后的帧仍会不带排除地拼进去；滚动循环对抓取错误是睡眠后重试。本轮没有给它新的停止原因。

## 实跑测试

`.cargo/config.toml` 已固定 `jobs = 1` 和 `target-dir = D:\YourAllIn1Helper-worktrees\capture-review\target`。冷 target 只编了下面两个 lib 测试。日志在 `.runtime_data/windows-capture-review/`。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p os-hal --lib default_capture_exclusion -- --test-threads=8` | **2 passed**，0 failed，42 filtered out。`Finished test profile in 14m 26s`。日志 `os-hal-default-exclusion.log` |
| `cargo test -p mod-screenshot --lib test_gif_ -- --test-threads=8` | **13 passed**，0 failed，41 filtered out。`Finished test profile in 28.35s`，测试本身 0.31s。日志 `mod-screenshot-gif.log` |

新增用例：

- `default_capture_exclusion_rejects_handle_wider_than_u32`：`0x0000_7FFE_1234_5678` 返回「无法用 32 位窗口号表示」，且 `capture_region` / `capture_region_below` 都没有被调用。
- `default_capture_exclusion_keeps_u32_window_ids`：`u32::MAX` 仍进入 `capture_region_below`；`0` 和 `None` 仍是普通抓取。
- `test_gif_recording_stops_when_registered_handle_changes`：首帧后登记从 11 换成 22。循环只抓了带 11 的那一帧，错误含「句柄已更换」，临时文件被删除。注入捕获器对后续调用仍返回成功帧，用来证明不是抓屏失败伪装出来的停止。
- `test_gif_recording_stops_when_registered_handle_disappears`：同上，登记变为 `None`，错误含「排除已失效」。
- `test_gif_recording_accepts_stable_registered_handle`：登记保持 5 时两帧都带 5，并留下预览文件。
- `test_gif_recording_keeps_explicit_window_id_when_registry_changes`：显式窗口号 99 在登记被清空后仍两帧都是 99，录制完成。这是 macOS `CGWindowID` 路径。
- `test_gif_recording_rejects_overflow_exclusion_without_partial_file`：不覆盖默认 `capture_region_excluding`，登记 64 位句柄。错误含「32 位」，`capture_region` 调用次数为 0，没有半成品。

既有 GIF 黑帧、墨迹合成、取消删除和原生抓取失败用例在同一过滤里通过。编译时 `os-hal` 的 `selection/mod.rs:84` 有一条原有的 `unreachable_expression` 警告，本轮没有改那个文件。

## 未完成

下面这些本轮没有执行，不能记为通过或失败。主目录暖缓存上的完整测试由 Codex 统一跑，本工作树没有再开编译。

- `cargo test --workspace` 以及其余 crate
- Tauri 桌面构建
- 真实 `SetWindowDisplayAffinity`、原生窗口、桌面 GIF / 长截图
- macOS 编译。`capture_below_window` 未改；显式窗口号只由注入测试锁住
- 混合 DPI 与 release 安装

上表两个过滤测试已经结束：`os-hal` 2 passed，`mod-screenshot` 的 `test_gif_` 13 passed。本轮启动的 Cargo 进程已退出，没有残留任务。`secure_services.rs` 上一次 `cargo fmt` 的无关换行已恢复，不在本次提交里。

## 其他限制

- 句柄变更、丢失和标识溢出只由注入捕获器验证，没有创建原生窗口。
- 长截图在登记中途被清空时仍会把没有排除的帧拼进去。这不是本轮移植的行为。
- `D:\YourAllIn1Helper-worktrees\gif` 的 4 个未提交文件保持原样。
- 编译日志里 `os-hal` 的 `selection/mod.rs:84` 有一条原有 `unreachable_expression` 警告，本轮没有改那个文件。
