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
