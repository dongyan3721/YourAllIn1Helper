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

    let samples = [
        (1, 4, 1, 4),
        (1, 2, 1, 3),
        (3, 4, 1, 3),
        (1, 2, 2, 3),
        (3, 4, 2, 3),
    ];
    let mut successful_chains = 0;
    for (x_num, x_den, y_num, y_den) in samples {
        let x = window.x + (window.width as i64 * x_num / x_den) as i32;
        let y = window.y + (window.height as i64 * y_num / y_den) as i32;
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
                successful_chains += 1;
            }
            Err(HalError::Unsupported(reason)) => {
                eprintln!("UIA sample ({x}, {y}) unsupported: {reason}");
            }
            Err(other) => panic!("unexpected Windows UIAutomation error: {other}"),
        }
    }
    eprintln!(
        "Windows UIAutomation returned {successful_chains} valid chain(s) across {} sample point(s)",
        samples.len()
    );
    assert!(
        successful_chains > 0,
        "requested QA application had no usable UIA control chain"
    );
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
