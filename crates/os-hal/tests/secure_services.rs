//! 剪贴板所有权与凭据库契约测试。
//!
//! 默认只运行显式测试适配器；写入真实系统剪贴板或登录钥匙串的原生测试标记为 ignored，
//! 需在桌面会话中显式运行：`cargo test -p os-hal --test secure_services -- --ignored`。
use os_hal::{ClipboardOwnership, MockSecureServices, SecretStore};

#[test]
fn clear_only_when_clipboard_still_holds_our_write() {
    let svc = MockSecureServices::new();
    let token = svc.write_concealed_text("fictional-secret-001").unwrap();
    assert_eq!(
        svc.clipboard_snapshot(),
        (Some("fictional-secret-001".into()), true)
    );
    assert!(svc.clear_if_unchanged(&token).unwrap());
    assert_eq!(svc.clipboard_snapshot(), (None, false));
    // 已清除后再次清除：序号已变化，不重复操作。
    assert!(!svc.clear_if_unchanged(&token).unwrap());
}

#[test]
fn later_user_copy_is_never_cleared_even_with_identical_text() {
    let svc = MockSecureServices::new();
    let token = svc.write_concealed_text("same-text").unwrap();
    // 用户随后复制了完全相同的文本：序号变化，必须保留用户内容。
    svc.simulate_external_copy("same-text");
    assert!(!svc.clear_if_unchanged(&token).unwrap());
    assert_eq!(svc.clipboard_snapshot(), (Some("same-text".into()), false));
}

#[test]
fn mock_secret_store_round_trip_and_unavailable_reporting() {
    let store = MockSecureServices::new();
    assert!(store.is_available());
    assert_eq!(store.get_secret("svc", "acct").unwrap(), None);
    store.set_secret("svc", "acct", b"fictional-key").unwrap();
    assert_eq!(
        store.get_secret("svc", "acct").unwrap().as_deref(),
        Some(&b"fictional-key"[..])
    );
    store.delete_secret("svc", "acct").unwrap();
    store.delete_secret("svc", "acct").unwrap();
    assert_eq!(store.get_secret("svc", "acct").unwrap(), None);

    let missing = MockSecureServices::without_secret_store();
    assert!(!missing.is_available());
    assert!(missing.get_secret("svc", "acct").is_err());
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "写入并删除登录钥匙串中的专用测试条目，需显式运行"]
fn native_keychain_round_trip_uses_dedicated_test_item() {
    let native = os_hal::NativeOsHal::new();
    let service = "com.allinone.desktop.test.secure-services";
    let account = format!("roundtrip-{}", std::process::id());
    assert!(native.is_available());
    assert_eq!(native.get_secret(service, &account).unwrap(), None);
    native
        .set_secret(service, &account, b"fictional-test-value")
        .unwrap();
    assert_eq!(
        native.get_secret(service, &account).unwrap().as_deref(),
        Some(&b"fictional-test-value"[..])
    );
    native.delete_secret(service, &account).unwrap();
    assert_eq!(native.get_secret(service, &account).unwrap(), None);
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "会覆盖真实系统剪贴板内容，需显式运行"]
fn native_pasteboard_clears_only_unchanged_content() {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    let native = os_hal::NativeOsHal::new();
    let token = native
        .write_concealed_text("fictional-clipboard-001")
        .unwrap();
    let pasteboard = NSPasteboard::generalPasteboard();
    // SAFETY: AppKit 导出的常量字符串。
    let text_type = unsafe { NSPasteboardTypeString };
    assert_eq!(
        pasteboard.stringForType(text_type).map(|s| s.to_string()),
        Some("fictional-clipboard-001".to_string())
    );
    assert!(native.clear_if_unchanged(&token).unwrap());
    assert!(pasteboard.stringForType(text_type).is_none());

    // 写入后被「其他程序」修改：不清除。
    let token = native
        .write_concealed_text("fictional-clipboard-002")
        .unwrap();
    pasteboard.clearContents();
    pasteboard.setString_forType(
        &objc2_foundation::NSString::from_str("user-content"),
        text_type,
    );
    assert!(!native.clear_if_unchanged(&token).unwrap());
    assert_eq!(
        pasteboard.stringForType(text_type).map(|s| s.to_string()),
        Some("user-content".to_string())
    );
    pasteboard.clearContents();
}
