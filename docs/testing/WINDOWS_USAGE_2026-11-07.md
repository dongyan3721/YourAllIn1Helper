# Windows OS HAL 系统使用状态能力开发与验证报告（2026-11-07）

## 1. 任务背景与执行环境

- **认领任务**：Windows 系统使用状态（Usage）能力迁移与第二轮审查反馈修复
- **工作目录**：`C:/Users/xcxix/PyCharmProjects/YourAllIn1Helper-windows-usage`
- **对应分支**：`codex/windows-usage`
- **基线版本**：`c7c6a4d fix(windows): establish tested desktop development baseline`
- **所属模块边界**：`crates/os-hal/src/platform/usage/**`、`crates/os-hal/Cargo.toml`（Windows 特性增补）及对应单元测试与能力测试；严禁越界修改 `mod-password-vault`、`secure`、`input` 或 `regions`。

---

## 2. 核心系统能力实现与第二轮审查修复明细

### 2.1 原生锁屏/睡眠/屏幕事件监控（`WinUsageEventMonitor`）
- **统一生命周期锁（Lifecycle Lock）**：
  - 引入 `lifecycle: Mutex<Option<RunningInstance>>` 统一管理后台工作线程句柄与顶层隐藏窗口句柄；
  - 消除旧实现中 `running` 原子标志与独立锁可能引发的“消息循环在 `WM_CLOSE` 被消费前退出留下孤儿窗口”及并发 `stop` 漏掉新 HWND 导致永久 `join` 等待的问题；
  - 重复调用 `start()` 保持幂等；未启动时调用 `stop()` 安全无操作。
- **严格三态处理 `GetMessageW` 返回值**：
  - 严格依据微软 Win32 文档处理 `GetMessageW` 三态：`ret.0 > 0` 派发消息，`ret.0 == 0`（收到 `WM_QUIT`）正常跳出循环，`ret.0 == -1`（系统调用失败）退出循环；禁止使用 `.as_bool()` 误把 `-1` 判定为继续循环。
- **同线程销毁与窗口类注销**：
  - 严格遵守 Win32 契约，在创建窗口的工作线程内调用 `DestroyWindow` 与 `UnregisterClassW`；
  - 涵盖所有退出路径：类创建失败、窗口创建失败、通知注册失败、ready 接收端已退出、正常 `WM_CLOSE` 退出以及 `GetMessageW` 错误退出。
- **系统通知注册失败显式报错与全量回滚**：
  - `WTSRegisterSessionNotification` 失败立即回滚销毁窗口与注销类，返回错误；
  - `RegisterSuspendResumeNotification` 失败注销已注册的会话通知并回收资源；
  - 会话显示通知失败回滚所有已注册通知并回收窗口与类。
- **单一权威会话显示通知源**：
  - 针对 Windows 10+ 优先注册 `GUID_SESSION_DISPLAY_STATUS` 作为单一权威显示源；仅在不支持时回退至 `GUID_CONSOLE_DISPLAY_STATE`；
  - 杜绝多通知源同时注册并竞争覆盖同一布尔值导致旧状态最后写入的问题。
- **状态重置（`UsageActivityState::reset`）**：
  - 监视器启动与停止时均重置状态机，清除陈旧事件状态（如历史 `ScreenLocked`），防止重启后旧锁屏事件永久遮蔽真实的未锁屏轮询结果。

### 2.2 可靠的锁屏状态严格轮询（`usage_screen_locked_windows`）
- **严格区分会话连接与锁定状态**：
  - 弃用不精确的 `WTSConnectState == WTSActive`；
  - 调用 `WTSQuerySessionInformationW(WTSSessionInfoEx)` 解析 `WTSINFOEXW`（`Level == 1`）中的 `SessionFlags`；
  - 若 `SessionFlags == WTS_SESSIONSTATE_LOCK` 明确为锁定；
  - 若为 `WTS_SESSIONSTATE_UNKNOWN` 或未知系统标志，保守返回系统错误以触发上层暂停计时。
- **交叉验证桌面切换权限**：
  - 调用 `OpenInputDesktop(DESKTOP_SWITCHDESKTOP)`；
  - 锁屏/受保护安全桌面返回 `ERROR_ACCESS_DENIED` (5 / 0x80070005) 判定为锁屏；
  - 能够成功打开输入桌面且 `SessionFlags == WTS_SESSIONSTATE_UNLOCK` 时判定为未锁屏。
- **严格 RAII 释放内存**：
  - 引入 `WtsBufferGuard` 保证在所有分支退出、提前返回或异常路径均调用 `WTSFreeMemory`。

### 2.3 真实 Windows Core Audio API 默认输出音频活动探测（`usage_audio_active_windows`）
- **COM 初始化与管理**：
  - 采用 `CoInitializeEx(None, COINIT_MULTITHREADED)` 并配合 `ComGuard` 仅在本函数初始化时配对卸载，兼容已初始化为 STA 或 MTA 的外部调用线程。
- **端点获取与无设备边界**：
  - 调用 `IMMDeviceEnumerator::GetDefaultAudioEndpoint(eRender, eConsole)`；
  - 找不到默认设备时（`E_NOTFOUND` 0x80070490 / 0x80070002）与 macOS 保持一致返回 `Ok(false)`。
- **双重真实探测管线与错误保留**：
  - `IAudioSessionManager2`：通过 `GetSessionEnumerator()` 遍历会话，若检测到 `AudioSessionStateActive` 立即判定为活跃；若遍历或 `GetSession`/`GetState` 调用发生系统错误且未确认活跃流，**严格保留底层错误并向上传播，绝不吞错冒充 `Ok(false)`**。
  - `IAudioMeterInformation`：通过 `GetPeakValue()` 检查主渲染电平（`peak > 0.0001`）。
- **契约守卫与合并决策**：
  - 拆出纯逻辑决策函数 `evaluate_windows_audio_active`；
  - 任一探测证实有音频流返回 `Ok(true)`；
  - 两路皆正常且无活跃返回 `Ok(false)`；
  - 一路正常无活跃、一路不支持返回 `Ok(false)`；
  - **两路探测全部遭遇 COM/系统调用错误时，严禁吞错冒充 false，必须返回真实错误（`HalError::NativeCallFailed`）**。

### 2.4 测试分层与纯逻辑/真机隔离
- **纯单元测试（`crates/os-hal/tests/unit/usage_events.rs`）**：
  - 绝不依赖 Windows 用户桌面会话或硬件设备；
  - 全覆盖测试锁屏决策逻辑 `evaluate_windows_screen_locked` 所有分支（已锁定、已解锁、受限凭据桌面、未知标志、WTS 失败）；
  - 全覆盖测试音频决策逻辑 `evaluate_windows_audio_active` 所有分支（活跃、无活跃、单路错误、双路全错）；
  - 覆盖状态重置测试（`state_reset_clears_stale_events_and_restores_unlocked_polling`）与状态机重启/故障恢复纯逻辑测试（`state_machine_restart_and_failure_recovery_pure_logic`）。
- **原生桌面专项测试（`crates/os-hal/tests/capabilities.rs`）**：
  - 真实桌面监视器启停（`windows_usage_event_monitor_start_stop_is_idempotent`）与真实音频探测（`windows_audio_activity_probe_returns_a_real_boolean`）按协调规范显式标注 `#[ignore]`，支持通过 `-- --ignored` 单独执行验收。

---

## 3. 测试命令与执行结果

### 3.1 `cargo check -p os-hal -j 2`
- **结果**：编译通过（Exit Code 0，0 error，无本模块 warning）。

### 3.2 `cargo test -p os-hal -j 2`
```text
running 12 tests
test platform::usage::tests::out_of_order_and_rapid_state_transitions ... ok
test platform::usage::tests::polling_result_remains_authoritative_when_events_are_incomplete ... ok
test platform::usage::tests::sleep_wake_requires_both_workspace_and_screen_wake ... ok
test tests::test_native_windows_list ... ignored, requires a logged-in desktop session
test platform::usage::tests::state_machine_restart_and_failure_recovery_pure_logic ... ok
test platform::usage::tests::state_reset_clears_stale_events_and_restores_unlocked_polling ... ok
test platform::usage::tests::system_events_pause_and_only_resume_after_confirmed_unlock ... ok
test platform::usage::tests::unknown_state_and_failure_paths_are_conservative ... ok
test platform::usage::tests::windows_audio_active_decision_logic_covers_all_branches ... ok
test tests::test_mock_os_hal_window_topmost_and_paste ... ok
test tests::test_selection_extractor_contracts ... ok
test platform::usage::tests::windows_screen_locked_decision_logic_covers_all_branches ... ok

test result: ok. 11 passed; 0 failed; 1 ignored; finished in 0.00s

     Running tests\capabilities.rs
running 4 tests
test windows_active_app_watcher_reports_real_state ... ignored, requires a live desktop session with interactive windows; run explicitly for native QA
test windows_audio_activity_probe_returns_a_real_boolean ... ignored, requires live audio endpoint hardware/session; run explicitly for native QA
test windows_usage_event_monitor_start_stop_is_idempotent ... ignored, requires interactive desktop session for WTS and power broadcast window; run explicitly for native QA
test unimplemented_native_capabilities_do_not_return_mock_results ... ok

test result: ok. 1 passed; 0 failed; 3 ignored; finished in 0.03s

     Running tests\secure_services.rs
running 3 tests
test later_user_copy_is_never_cleared_even_with_identical_text ... ok
test clear_only_when_clipboard_still_holds_our_write ... ok
test mock_secret_store_round_trip_and_unavailable_reporting ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; finished in 0.00s
```

### 3.3 原生桌面能力显式测试（`cargo test -p os-hal --test capabilities -- --ignored`）
```text
running 3 tests
test windows_active_app_watcher_reports_real_state ... ok
test windows_usage_event_monitor_start_stop_is_idempotent ... ok
test windows_audio_activity_probe_returns_a_real_boolean ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; finished in 0.02s
```

### 3.4 业务模块集成回归测试（`cargo test -p mod-app-usage -j 2`）
```text
running 29 tests
test config::tests::config_normalizes_out_of_range_values ... ok
test config::tests::category_accepts_only_known_values ... ok
test config::tests::legacy_config_without_audio_flag_defaults_to_enabled ... ok
test store::tests::date_ranges_are_validated_and_filled ... ok
test tests::capabilities_report_audio_limit_when_probe_is_unsupported ... ok
test tests::audio_probe_failure_keeps_plain_idle_pause ... ok
test tests::disabling_audio_exception_restores_plain_idle_pause ... ok
test tests::paused_samples_do_not_create_or_write_empty_sessions ... ok
test tests::reliability::half_hour_timezone_splits_at_local_midnight ... ok
test tests::focus_switch_flushes_and_splits_across_hour_boundary ... ok
test tests::reliability::disabling_titles_scrubs_the_active_session_and_pending_metadata ... ok
test tests::reliability::accepted_wall_clock_drift_does_not_change_billed_duration ... ok
test tests::lock_and_sleep_take_priority_over_audio_exception ... ok
test tests::active_time_is_accumulated_with_monotonic_clock ... ok
test tests::idle_and_lock_pause_without_recording ... ok
test tests::audio_playback_keeps_idle_time_active ... ok
test tests::ignored_apps_stop_counting_but_keep_history ... ok
test tracker::tests::split_interval_crosses_hour_and_day_boundaries ... ok
test tracker::tests::split_interval_handles_dst_like_offset_changes_without_losing_time ... ok
test tracker::tests::split_interval_ignores_reversed_ranges ... ok
test tests::reliability::failed_session_write_is_atomic_and_retried_once ... ok
test tests::repeated_enable_disable_starts_and_stops_event_monitor ... ok
test tests::sleep_wake_gap_is_not_counted ... ok
test tests::sessions_list_reports_day_sessions_and_hides_titles_when_disabled ... ok
test tests::window_titles_can_be_disabled_before_persisting ... ok
test tests::reliability::unavailable_idle_or_lock_state_never_counts_as_active_usage ... ok
test tests::restart_does_not_recount_or_add_offline_time ... ok
test tests::module_shutdown_flushes_pending_usage_and_stops_sampler ... ok
test tests::reliability::failed_disable_recovers_sampling_and_preserves_unflushed_usage ... ok

test result: ok. 29 passed; 0 failed; 0 ignored; finished in 2.03s
```

---

## 4. 未测项与已知物理限制

1. **真实物理电源睡眠与合盖唤醒**：在持续自动化构建会话中无法模拟操作系统级物理睡眠（S3/Modern Standby），需依赖桌面实机用户手动合盖验收；
2. **多音频端点热插拔切换**：默认输出设备在播放中动态断开/切换 USB 耳机流的极端过渡态需物理硬件测试。

---

## 5. 变更文件清单与提交哈希

- `crates/os-hal/Cargo.toml`：增补 `Win32_System_RemoteDesktop`, `Win32_System_Power`, `Win32_Media_Audio`, `Win32_Media_Audio_Endpoints`, `Win32_System_Com_StructuredStorage`。
- `crates/os-hal/src/platform/usage/mod.rs`：接入 Windows 事件监视器、导出决策纯函数并实现 `UsageActivityState::reset`。
- `crates/os-hal/src/platform/usage/windows.rs`：实现 `WTSSessionInfoEx` 校验、`WtsBufferGuard`、`evaluate_windows_screen_locked` 与真实 Core Audio 双路探测（错误严格保留）`evaluate_windows_audio_active`。
- `crates/os-hal/src/platform/usage/windows_events.rs`：实现统一生命周期锁（`lifecycle`）、标准 Win32 消息循环（GetMessageW 三态）、同线程销毁/注销、注册失败回滚与 Windows 10+ 单一权威显示源监控。
- `crates/os-hal/tests/unit/usage_events.rs`：纯逻辑单元测试，覆盖锁屏/音频所有决策分支与状态机重置/恢复。
- `crates/os-hal/tests/capabilities.rs`：原生能力测试（Windows 桌面监视器与音频探测标注 `#[ignore]`）。
- `docs/testing/WINDOWS_USAGE_2026-11-07.md`：本交付报告。

**所属分支**：`codex/windows-usage`
**基线版本**：`c7c6a4d fix(windows): establish tested desktop development baseline`
**本次提交**：`1025d1960ee819d93e7a347b7b8ffba147ab68b8`（`feat(os-hal): implement Windows usage state and audio activity probes`）
