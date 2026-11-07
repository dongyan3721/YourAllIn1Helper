//! 密码本复制必须经过敏感标记、共享租约与序号清理；仅用显式内存适配器。
use super::*;
use os_hal::{ClipboardOwnership, ClipboardWriteToken, HalError, HalResult, MockSecureServices};

#[test]
fn secret_copy_requires_adapter_and_holds_suppression_until_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    assert!(!host.password_vault.service().status().secret_copy_available);
    let board = Arc::new(MockSecureServices::new());
    host.set_vault_clipboard(board.clone());
    assert!(host.password_vault.service().status().secret_copy_available);
    let ctx = host.kernel().context();
    let pending = write_secret_with_lease(ctx, board.clone(), "fictional-secret").unwrap();
    assert!(ctx.event_bus.is_clipboard_suppressed());
    assert_eq!(
        board.clipboard_snapshot(),
        (Some("fictional-secret".into()), true)
    );
    drop(pending);
    assert_eq!(board.clipboard_snapshot(), (None, false));
    assert!(!ctx.event_bus.is_clipboard_suppressed());
}

#[test]
fn expiry_keeps_later_user_copy_even_if_text_is_identical() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let board = Arc::new(MockSecureServices::new());
    let pending =
        write_secret_with_lease(host.kernel().context(), board.clone(), "same text").unwrap();
    board.simulate_external_copy("same text");
    drop(pending);
    assert_eq!(
        board.clipboard_snapshot(),
        (Some("same text".into()), false)
    );
    assert!(!host.kernel().context().event_bus.is_clipboard_suppressed());
}

#[test]
fn failed_sensitive_write_releases_suppression() {
    struct Unavailable;
    impl ClipboardOwnership for Unavailable {
        fn write_concealed_text(&self, _: &str) -> HalResult<ClipboardWriteToken> {
            Err(HalError::Unsupported("test clipboard unavailable"))
        }
        fn clear_if_unchanged(&self, _: &ClipboardWriteToken) -> HalResult<bool> {
            panic!("failed write cannot create a cleanup token")
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let ctx = host.kernel().context();
    assert!(write_secret_with_lease(ctx, Arc::new(Unavailable), "fictional").is_err());
    assert!(!ctx.event_bus.is_clipboard_suppressed());
}

#[test]
fn normal_exit_clears_only_a_still_owned_secret() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    let board = Arc::new(MockSecureServices::new());
    host.set_vault_clipboard(board.clone());
    *host.last_vault_copy.lock() = Some(board.write_concealed_text("fictional").unwrap());
    host.clear_vault_clipboard();
    assert_eq!(board.clipboard_snapshot(), (None, false));
    *host.last_vault_copy.lock() = Some(board.write_concealed_text("fictional").unwrap());
    board.simulate_external_copy("fictional");
    host.clear_vault_clipboard();
    assert_eq!(
        board.clipboard_snapshot(),
        (Some("fictional".into()), false)
    );
}
