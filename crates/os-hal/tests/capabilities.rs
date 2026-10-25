//! 平台能力边界回归测试，无需桌面权限或真实系统输入。
use os_hal::{BiometricAuthenticator, HalError, NativeOsHal, OcrProvider, SelectionExtractor};
#[test]
fn unimplemented_native_capabilities_do_not_return_mock_results() {
    let native = NativeOsHal::new();
    assert!(!native.is_biometric_available());
    assert!(matches!(
        native.verify_identity("test"),
        Err(HalError::Unsupported(_))
    ));
    assert!(matches!(
        native.extract_selected_text(),
        Err(HalError::Unsupported(_))
    ));
    assert!(matches!(
        native.recognize_rgba(&os_hal::CapturedFrame {
            width: 1,
            height: 1,
            scale_factor_milli: 1000,
            rgba_bytes: vec![0; 4],
        }),
        Err(HalError::Unsupported(_))
    ));
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
