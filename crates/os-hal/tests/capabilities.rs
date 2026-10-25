//! 平台能力边界回归测试，无需桌面权限或真实系统输入。
use os_hal::{BiometricAuthenticator, HalError, NativeOsHal, OcrProvider};
#[test]
fn unimplemented_native_capabilities_do_not_return_mock_results() {
    let native = NativeOsHal::new();
    assert!(!native.is_biometric_available());
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
    if cfg!(target_os = "macos") {
        // macOS 已接入 Vision：空白图只能得到空结果或真实错误，绝不能出现示例文本。
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
