//! 平台能力边界回归测试，无需桌面权限或真实系统输入。
use os_hal::{BiometricAuthenticator, HalError, NativeOsHal, OcrProvider};
#[test]
fn unimplemented_native_capabilities_do_not_return_mock_results() {
    let native = NativeOsHal::new();
    // 探测可以因硬件而不同，但不得弹窗，也不得用认证布尔值代替钥匙串解钥。
    let _available = native.is_biometric_available();
    assert!(matches!(
        native.verify_identity("test"),
        Err(HalError::Unsupported(_))
    ));
    // 原生选区需要显式桌面验收，自动测试不得读取用户真实选区。
    let blank = os_hal::CapturedFrame {
        width: 1,
        height: 1,
        scale_factor_milli: 1000,
        rgba_bytes: vec![0; 4],
    };
    let ocr = native.recognize_rgba(&blank);
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        // 已接入平台的空白图只能得到空结果或真实错误，绝不能出现示例文本。
        if let Ok(result) = ocr {
            assert!(result.full_text.is_empty() && result.lines.is_empty());
        }
    } else {
        assert!(matches!(ocr, Err(HalError::Unsupported(_))));
    }
    // 尺寸与缓冲区不一致必须拒绝，不能读越界内存。
    assert!(native
        .recognize_rgba(&os_hal::CapturedFrame {
            width: 4,
            height: 4,
            scale_factor_milli: 1000,
            rgba_bytes: vec![0; 4],
        })
        .is_err());
}
#[cfg(target_os = "macos")]
#[test]
fn macos_capture_permission_preflight_only_reports_granted_or_denied() {
    use os_hal::ScreenCapturer;
    // 预检不会弹窗；未授权时必须是带指引的 PermissionDenied，而不是假装成功或其他错误。
    match NativeOsHal::new().check_capture_permission() {
        Ok(()) => {}
        Err(HalError::PermissionDenied {
            capability,
            guidance,
        }) => {
            assert_eq!(capability, "screen_recording");
            assert!(!guidance.is_empty());
        }
        Err(other) => panic!("unexpected preflight error: {other}"),
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_external_topmost_reports_unsupported_without_touching_windows() {
    use os_hal::WindowController;
    let native = NativeOsHal::new();
    assert!(!native.supports_external_topmost());
    assert!(matches!(
        native.set_external_window_topmost(0, true),
        Err(HalError::Unsupported(_))
    ));
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires a live desktop session with available CGSession state; run explicitly for native QA"]
fn macos_active_app_watcher_reports_real_state() {
    use os_hal::ActiveAppWatcher;
    let native = NativeOsHal::new();
    let idle = native.system_idle_seconds().expect("idle detection");
    assert!(idle < 86_400);
    let _locked = native.is_screen_locked().expect("screen lock detection");
    let app = native.current_active_app().expect("frontmost application");
    assert!(!app.app_id.trim().is_empty());
    assert!(!app.app_name.trim().is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_audio_activity_probe_returns_a_real_boolean() {
    use os_hal::ActiveAppWatcher;
    // 只查询默认输出设备是否在运行，不读取音频内容，也不需要隐私授权。
    match NativeOsHal::new().default_output_audio_active() {
        Ok(_) => {}
        Err(HalError::Unsupported(capability)) => {
            panic!("CoreAudio probe unexpectedly unsupported: {capability}")
        }
        Err(other) => panic!("CoreAudio probe failed: {other}"),
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires an AppKit process with a live desktop session; run explicitly for native QA"]
fn macos_usage_event_monitor_start_stop_is_idempotent() {
    use os_hal::ActiveAppWatcher;
    let native = NativeOsHal::new();
    native.start_usage_event_monitor().expect("start monitor");
    native
        .start_usage_event_monitor()
        .expect("start monitor twice");
    native.stop_usage_event_monitor();
    native.stop_usage_event_monitor();
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "需要桌面会话；读取前台外部窗口的无障碍层级（只读），显式运行"]
fn macos_ui_hierarchy_reports_real_chain_or_permission_denied() {
    use os_hal::{RegionDetector, WindowController};
    let native = NativeOsHal::new();
    let Some(window) = native
        .list_external_windows()
        .unwrap_or_default()
        .into_iter()
        .find(|w| w.width > 100 && w.height > 100)
    else {
        return;
    };
    let (x, y) = (
        window.x + window.width as i32 / 2,
        window.y + window.height as i32 / 2,
    );
    match native.query_ui_hierarchy_at_point(x, y) {
        Ok(chain) => {
            println!(
                "AX chain at ({x},{y}) in {}: {:?}",
                window.process_name, chain
            );
            for r in &chain {
                assert!(r.width >= 4 && r.height >= 4);
            }
        }

        Err(HalError::PermissionDenied { capability, .. }) => {
            assert_eq!(capability, "accessibility")
        }
        Err(other) => panic!("unexpected AX error: {other}"),
    }
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "需要真实桌面；只读查询外部窗口 UIAutomation 控件链，显式运行"]
fn windows_ui_hierarchy_returns_real_chain_for_requested_qa_process() {
    use os_hal::{RegionDetector, WindowController};
    let native = NativeOsHal::new();
    let requested_process = std::env::var("AIO_UIA_SMOKE_PROCESS")
        .expect("set AIO_UIA_SMOKE_PROCESS to a visible QA application process name");
    let Some(window) = native
        .list_external_windows()
        .expect("enumerate external windows")
        .into_iter()
        .find(|window| {
            window.width > 100
                && window.height > 100
                && window
                    .process_name
                    .to_lowercase()
                    .contains(&requested_process.to_lowercase())
        })
    else {
        panic!("requested UIA QA application is not visible");
    };
    // 先尝试把目标放到前台。置顶窗口仍可能挡住采样点，后面按真实 Z 序剔除。
    let _ = native.activate_external_window(window.handle);
    eprintln!(
        "UIA QA window pid={} process={} title={} rect={},{} {}x{} handle={}",
        window.process_id,
        window.process_name,
        window.title,
        window.x,
        window.y,
        window.width,
        window.height,
        window.handle
    );

    let samples = [
        (1, 4, 1, 4),
        (1, 2, 1, 3),
        (3, 4, 1, 3),
        (1, 2, 2, 3),
        (3, 4, 2, 3),
    ];
    let mut successful_chains = 0;
    let mut covered_points = 0;
    let mut queried_points = 0;
    for (x_num, x_den, y_num, y_den) in samples {
        let x = window.x + (window.width as i64 * x_num / x_den) as i32;
        let y = window.y + (window.height as i64 * y_num / y_den) as i32;
        match topmost_external_window_at(x, y) {
            Some(hit) if hit.hwnd == window.handle as isize => {}
            Some(hit) => {
                covered_points += 1;
                eprintln!(
                    "UIA sample ({x}, {y}) covered by hwnd={} pid={} class={} title={}",
                    hit.hwnd, hit.process_id, hit.class_name, hit.title
                );
                continue;
            }
            None => {
                covered_points += 1;
                eprintln!("UIA sample ({x}, {y}) has no external top-level window");
                continue;
            }
        }
        queried_points += 1;
        match native.query_ui_hierarchy_at_point(x, y) {
            Ok(chain) => {
                assert!(chain.len() >= 2, "控件和顶层窗口必须都真实返回");
                assert_eq!(
                    chain.last().unwrap().level,
                    app_core::RegionHierarchyLevel::TopLevelWindow
                );
                assert!(chain
                    .windows(2)
                    .all(|pair| pair[0].parent_id == Some(pair[1].region_id)));
                let root = chain.last().unwrap();
                assert!(
                    root.x <= x && x < root.x + root.width as i32,
                    "top-level bounds must contain the sample point"
                );
                assert!(
                    root.y <= y && y < root.y + root.height as i32,
                    "top-level bounds must contain the sample point"
                );
                successful_chains += 1;
            }
            Err(HalError::Unsupported(reason)) => {
                eprintln!("UIA sample ({x}, {y}) unsupported: {reason}");
            }
            Err(other) => panic!("unexpected Windows UIAutomation error: {other}"),
        }
    }
    eprintln!(
        "Windows UIAutomation returned {successful_chains} valid chain(s) across {queried_points} unobstructed sample point(s); {covered_points} point(s) were covered"
    );
    assert!(
        queried_points > 0,
        "requested QA window is covered at every sample point; uncover it before treating UIA as failed"
    );
    assert!(
        successful_chains > 0,
        "requested QA application had no usable UIA control chain"
    );
}

#[cfg(target_os = "windows")]
struct TopmostWindowHit {
    hwnd: isize,
    process_id: u32,
    class_name: String,
    title: String,
}

#[cfg(target_os = "windows")]
fn topmost_external_window_at(x: i32, y: i32) -> Option<TopmostWindowHit> {
    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{HWND, LPARAM, RECT},
            UI::{
                HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
                WindowsAndMessaging::{
                    EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW,
                    GetWindowThreadProcessId, IsWindowVisible,
                },
            },
        },
    };

    struct Search {
        x: i32,
        y: i32,
        own_pid: u32,
        hit: Option<TopmostWindowHit>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        let state = &mut *(data.0 as *mut Search);
        if state.hit.is_some() || !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == 0 || process_id == state.own_pid {
            return BOOL(1);
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return BOOL(1);
        }
        if state.x < rect.left
            || state.x >= rect.right
            || state.y < rect.top
            || state.y >= rect.bottom
        {
            return BOOL(1);
        }
        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf);
        let mut title_buf = [0u16; 512];
        let title_len = GetWindowTextW(hwnd, &mut title_buf);
        state.hit = Some(TopmostWindowHit {
            hwnd: hwnd.0 as isize,
            process_id,
            class_name: String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize]),
            title: String::from_utf16_lossy(&title_buf[..title_len.max(0) as usize]),
        });
        BOOL(0)
    }

    // 与产品查询使用同一套物理像素，避免遮挡判断和 UIAutomation 命中的不是同一个窗口。
    unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let mut state = Search {
        x,
        y,
        own_pid: std::process::id(),
        hit: None,
    };
    let enumerated =
        unsafe { EnumWindows(Some(visit), LPARAM(&mut state as *mut Search as isize)) };
    if state.hit.is_none() {
        enumerated.ok();
    }
    state.hit
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires live audio endpoint hardware/session; run explicitly for native QA"]
fn windows_audio_activity_probe_returns_a_real_boolean() {
    use os_hal::ActiveAppWatcher;
    // 只查询默认输出设备是否有音频流在运行，不读取音频内容，也不需要额外隐私授权。
    match NativeOsHal::new().default_output_audio_active() {
        Ok(_) => {}
        Err(HalError::Unsupported(capability)) => {
            panic!("Windows Core Audio probe unexpectedly unsupported: {capability}")
        }
        Err(other) => panic!("Windows Core Audio probe failed: {other}"),
    }
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires interactive desktop session for WTS and power broadcast window; run explicitly for native QA"]
fn windows_usage_event_monitor_start_stop_is_idempotent() {
    use os_hal::ActiveAppWatcher;
    let native = NativeOsHal::new();
    native.start_usage_event_monitor().expect("start monitor");
    native
        .start_usage_event_monitor()
        .expect("start monitor twice");
    native.stop_usage_event_monitor();
    native.stop_usage_event_monitor();
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires a live desktop session with interactive windows; run explicitly for native QA"]
fn windows_active_app_watcher_reports_real_state() {
    use os_hal::ActiveAppWatcher;
    let native = NativeOsHal::new();
    let idle = native.system_idle_seconds().expect("idle detection");
    assert!(idle < 86_400);
    let _locked = native.is_screen_locked().expect("screen lock detection");
    match native.current_active_app() {
        Ok(app) => {
            assert!(!app.app_id.trim().is_empty());
            assert!(!app.app_name.trim().is_empty());
        }
        Err(HalError::NotFound(_)) => {
            // 在无前台聚焦窗口的环境（如某些非交互式后台调用）返回 NotFound 符合预期
        }
        Err(err) => panic!("unexpected active app error: {err}"),
    }
}
