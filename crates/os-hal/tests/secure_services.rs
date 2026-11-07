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

#[test]
fn mock_secret_store_preserves_binary_secrets() {
    // 契约按二进制 blob 原样存取（含 NUL 与非 UTF-8 字节），不假定 UTF-8。
    let store = MockSecureServices::new();
    let blob: Vec<u8> = vec![0x00, 0x01, 0x41, 0xFF, 0x00, 0x7F, 0xFE];
    store
        .set_secret("com.allinone.desktop.test.binary", "blob", &blob)
        .unwrap();
    assert_eq!(
        store
            .get_secret("com.allinone.desktop.test.binary", "blob")
            .unwrap()
            .as_deref(),
        Some(&blob[..])
    );
    // 空秘密也是合法二进制。
    store
        .set_secret("com.allinone.desktop.test.binary", "empty", &[])
        .unwrap();
    assert_eq!(
        store
            .get_secret("com.allinone.desktop.test.binary", "empty")
            .unwrap()
            .as_deref(),
        Some(&[][..])
    );
}

/// Windows 系统集成验证（真实 Win32 API）：仅使用本应用独立虚构命名空间；
/// 会短暂覆盖系统剪贴板，全部显式 `#[ignore]`，需在桌面会话中运行：
/// `cargo test -p os-hal --test secure_services -- --ignored --test-threads=1`
#[cfg(target_os = "windows")]
mod windows_native {
    use os_hal::{clipboard_content_is_concealed, ClipboardOwnership, NativeOsHal, SecretStore};
    use std::sync::Mutex;
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::{SetLastError, HANDLE, HGLOBAL, HWND, WIN32_ERROR},
            System::{
                DataExchange::{
                    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
                    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
                },
                Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
                Ole::CF_UNICODETEXT,
            },
            UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE},
        },
    };

    /// 剪贴板真实测试互斥执行，避免并行用例互相抢剪贴板。
    /// 前一用例 panic 毒化锁时仍继续（用例之间不共享状态）。
    static CLIPBOARD_GATE: Mutex<()> = Mutex::new(());

    fn clipboard_gate() -> std::sync::MutexGuard<'static, ()> {
        CLIPBOARD_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 1×1 像素的最小合法 GIF89a（虚构测试数据）。
    const TINY_GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xFF, 0xFF, 0xFF, 0x21, 0xF9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3B,
    ];

    /// 测试侧独立的剪贴板会话：自建消息窗口作为 owner（NULL owner 会使写入失败）。
    struct TestClipboard {
        owner: HWND,
    }

    impl TestClipboard {
        fn open() -> Self {
            // SAFETY: 纯消息窗口，测试线程内创建并销毁。
            let owner = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("secure-services-test-owner"),
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    Some(HWND_MESSAGE),
                    None,
                    None,
                    None,
                )
            }
            .expect("创建测试用所有者窗口失败");
            unsafe { OpenClipboard(Some(owner)).expect("打开系统剪贴板失败") };
            Self { owner }
        }

        fn empty(&self) {
            // SAFETY: 剪贴板处于打开状态且 owner 有效。
            unsafe { EmptyClipboard().expect("清空系统剪贴板失败") };
        }
    }

    impl Drop for TestClipboard {
        fn drop(&mut self) {
            // SAFETY: 与本会话的打开/创建配对。
            unsafe {
                let _ = CloseClipboard();
                let _ = DestroyWindow(self.owner);
            }
        }
    }

    /// 测试侧把文本写入 `CF_UNICODETEXT`（模拟「其他程序/用户复制」）。
    fn raw_write_text(text: &str) {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = unsafe {
            std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2)
        };
        let board = TestClipboard::open();
        board.empty();
        // SAFETY: 合法 GMEM_MOVEABLE 句柄，写入后所有权移交剪贴板。
        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).expect("分配全局内存失败");
            let target = GlobalLock(handle);
            assert!(!target.is_null());
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
            SetLastError(WIN32_ERROR(0));
            let _ = GlobalUnlock(handle);
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0)))
                .expect("SetClipboardData 失败");
        }
    }

    /// 测试侧读取当前 `CF_UNICODETEXT` 文本（独立于被测实现）。
    fn raw_read_text() -> Option<String> {
        // 会话只用于保持剪贴板打开（Drop 时关闭）。
        let _board = TestClipboard::open();
        // SAFETY: 打开状态下读取剪贴板数据句柄。
        let handle = unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }.ok()?;
        let hglobal = HGLOBAL(handle.0);
        unsafe {
            let size = GlobalSize(hglobal);
            let view = GlobalLock(hglobal);
            if view.is_null() {
                return None;
            }
            let units = size / 2;
            let wide = std::slice::from_raw_parts(view.cast::<u16>(), units);
            let len = wide.iter().position(|&u| u == 0).unwrap_or(units);
            let text = String::from_utf16_lossy(&wide[..len]);
            SetLastError(WIN32_ERROR(0));
            let _ = GlobalUnlock(hglobal);
            Some(text)
        }
    }

    /// 测试侧清空剪贴板（用例收尾，避免残留虚构内容）。
    fn raw_clear() {
        let board = TestClipboard::open();
        board.empty();
    }

    /// 测试侧查询注册格式是否在剪贴板上。
    fn format_available(name: &str) -> bool {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: wide 为合法 NUL 结尾字符串。
        let format = unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) };
        format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok()
    }

    /// 测试侧读取自定义格式的字节内容。
    fn raw_read_format(name: &str) -> Option<Vec<u8>> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: wide 为合法 NUL 结尾字符串。
        let format = unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) };
        if format == 0 {
            return None;
        }
        // 会话只用于保持剪贴板打开（Drop 时关闭）。
        let _board = TestClipboard::open();
        // SAFETY: 打开状态下读取剪贴板数据句柄。
        let handle = unsafe { GetClipboardData(format) }.ok()?;
        let hglobal = HGLOBAL(handle.0);
        unsafe {
            let size = GlobalSize(hglobal);
            let view = GlobalLock(hglobal);
            if view.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(view.cast::<u8>(), size).to_vec();
            SetLastError(WIN32_ERROR(0));
            let _ = GlobalUnlock(hglobal);
            Some(bytes)
        }
    }

    #[test]
    #[ignore = "写入并删除凭据管理器中的专用测试条目（本应用虚构命名空间），需显式运行"]
    fn native_credential_manager_round_trip_uses_dedicated_test_item() {
        let native = NativeOsHal::new();
        assert!(native.is_available(), "Windows 凭据管理器应已接入");
        let service = "com.allinone.desktop.test.secure-services";
        let account = format!("roundtrip-{}", std::process::id());
        // 清理历史残留（不存在视为成功），保证可重复运行。
        native.delete_secret(service, &account).unwrap();
        assert_eq!(native.get_secret(service, &account).unwrap(), None);

        // 二进制秘密（含 NUL 与非 UTF-8 字节）原样存取。
        let payload: Vec<u8> = vec![0x00, 0x01, 0x41, 0xFF, 0x00, 0x7F, 0xFE];
        native.set_secret(service, &account, &payload).unwrap();
        assert_eq!(
            native.get_secret(service, &account).unwrap().as_deref(),
            Some(&payload[..])
        );

        // 非 ASCII 账户名（UTF-16 路径）往返。
        let account_zh = format!("账号-{}", std::process::id());
        native.delete_secret(service, &account_zh).unwrap();
        native.set_secret(service, &account_zh, &payload).unwrap();
        assert_eq!(
            native.get_secret(service, &account_zh).unwrap().as_deref(),
            Some(&payload[..])
        );

        // 清理：删除后不存在；重复删除不报错。
        native.delete_secret(service, &account).unwrap();
        native.delete_secret(service, &account_zh).unwrap();
        assert_eq!(native.get_secret(service, &account).unwrap(), None);
        assert_eq!(native.get_secret(service, &account_zh).unwrap(), None);
        native.delete_secret(service, &account).unwrap();
    }

    #[test]
    #[ignore = "写入并删除凭据管理器中的专用测试条目（本应用虚构命名空间），需显式运行"]
    fn native_credential_manager_rejects_out_of_namespace_service() {
        // 命名空间防护：绝不触碰本应用之外的凭据。
        let native = NativeOsHal::new();
        let error = native
            .set_secret("evil.example.service", "user", b"x")
            .unwrap_err();
        assert!(
            error.to_string().contains("命名空间"),
            "命名空间外的 service 必须被拒绝：{}",
            error
        );
    }

    #[test]
    #[ignore = "会覆盖真实系统剪贴板内容，需显式运行"]
    fn native_pasteboard_clears_only_unchanged_content() {
        let _gate = clipboard_gate();
        let native = NativeOsHal::new();
        let token = native
            .write_concealed_text("fictional-clipboard-001")
            .unwrap();
        assert_eq!(raw_read_text().as_deref(), Some("fictional-clipboard-001"));
        // 敏感标记必须齐备：剪贴板历史与云同步排除（独立读取验证，不走被测实现）。
        assert!(format_available("Exclude Clipboard Content From Monitor Processing"));
        assert!(format_available("Can Include In Clipboard History"));
        assert!(format_available("Can Upload To Cloud Clipboard"));
        // 探测函数与标记一致（监听器过滤依据）。
        assert!(clipboard_content_is_concealed());

        assert!(native.clear_if_unchanged(&token).unwrap());
        assert_eq!(raw_read_text(), None);

        // 写入后被「其他程序」修改：不清除。
        let token = native
            .write_concealed_text("fictional-clipboard-002")
            .unwrap();
        raw_write_text("user-content");
        assert!(!clipboard_content_is_concealed());
        assert!(!native.clear_if_unchanged(&token).unwrap());
        assert_eq!(raw_read_text().as_deref(), Some("user-content"));

        // 已清除的凭证再次清除：序号已变化，返回 false。
        let token = native.write_concealed_text("stale-token").unwrap();
        assert!(native.clear_if_unchanged(&token).unwrap());
        assert!(!native.clear_if_unchanged(&token).unwrap());

        raw_clear();
    }

    #[test]
    #[ignore = "会覆盖真实系统剪贴板内容，需显式运行"]
    fn native_pasteboard_keeps_user_copy_of_identical_text() {
        let _gate = clipboard_gate();
        let native = NativeOsHal::new();
        let token = native.write_concealed_text("same-text").unwrap();
        // 用户随后复制了完全相同的文本：系统序号变化，绝不能误清用户内容。
        raw_write_text("same-text");
        assert!(!native.clear_if_unchanged(&token).unwrap());
        assert_eq!(raw_read_text().as_deref(), Some("same-text"));
        raw_clear();
    }

    #[test]
    #[ignore = "会覆盖真实系统剪贴板内容，需显式运行"]
    fn native_animated_gif_clipboard_write_registers_gif_formats() {
        let _gate = clipboard_gate();
        os_hal::write_animated_gif_clipboard(TINY_GIF).expect("写入动态 GIF 失败");
        // 主格式必须可读回且字节一致。
        assert_eq!(raw_read_format("image/gif").as_deref(), Some(&TINY_GIF[..]));
        // 辅助短名尽力写入（失败不影响主格式），此处一并验证。
        assert_eq!(raw_read_format("GIF").as_deref(), Some(&TINY_GIF[..]));
        // GIF 复制不是敏感内容：不应带排除历史标记。
        assert!(!clipboard_content_is_concealed());
        raw_clear();
    }
}
