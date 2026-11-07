//! Windows 剪贴板所有权与凭据管理器子实现的纯逻辑回归单测。
//!
//! 全部用例不读写用户真实剪贴板、不访问凭据管理器、不弹权限框；
//! 真实系统集成验证在 `tests/secure_services.rs` 中以 `#[ignore]` 单独提供。
use super::{clipboard_win, secret_win, write_animated_gif_clipboard};
use crate::HalError;
use windows::Win32::{
    Foundation::{GlobalFree, HGLOBAL},
    System::Memory::{GlobalLock, GlobalSize},
    UI::WindowsAndMessaging::DestroyWindow,
};

#[test]
fn utf16_conversion_terminates_and_round_trips_non_ascii() {
    let wide = clipboard_win::utf16_with_nul("密码🔑abc").unwrap();
    // 结尾必须有 NUL 终止符。
    assert_eq!(wide.last(), Some(&0u16));
    // 去掉终止符后解码应与原文一致（覆盖 BMP 与代理对）。
    let round_trip = String::from_utf16(&wide[..wide.len() - 1]).unwrap();
    assert_eq!(round_trip, "密码🔑abc");
    // 空文本合法（只含终止符）。
    assert_eq!(clipboard_win::utf16_with_nul("").unwrap(), vec![0u16]);
}

#[test]
fn text_with_interior_nul_is_rejected_before_touching_clipboard() {
    let error = clipboard_win::utf16_with_nul("a\0b").unwrap_err();
    assert!(
        error.to_string().contains("NUL"),
        "必须拒绝会被 Windows 读者截断的文本：{}",
        error
    );
}

#[test]
fn concealed_marker_and_gif_format_names_are_the_documented_windows_values() {
    // 锁定微软文档化的注册格式名，防止改动时意外漂移导致秘密进入剪贴板历史。
    assert_eq!(
        clipboard_win::EXCLUDE_MONITOR_FORMAT,
        "Exclude Clipboard Content From Monitor Processing"
    );
    assert_eq!(
        clipboard_win::HISTORY_OPT_OUT_FORMAT,
        "Can Include In Clipboard History"
    );
    assert_eq!(
        clipboard_win::CLOUD_OPT_OUT_FORMAT,
        "Can Upload To Cloud Clipboard"
    );
    // GIF 主/辅格式名与 macOS 双 UTI 写入口径对应。
    assert_eq!(clipboard_win::GIF_PRIMARY_FORMAT, "image/gif");
    assert_eq!(clipboard_win::GIF_SECONDARY_FORMAT, "GIF");
}

#[test]
fn credential_target_names_are_confined_to_app_namespace() {
    // 合法：应用命名空间内的 service 映射为 {service}/{account}。
    assert_eq!(
        secret_win::credential_target_name("com.allinone.desktop.translator", "deepl").unwrap(),
        "com.allinone.desktop.translator/deepl"
    );
    // 拒绝命名空间外的 service，保证绝不触碰用户已有的其他凭据。
    let outside = secret_win::credential_target_name("deepl", "user").unwrap_err();
    assert!(outside.to_string().contains("命名空间"), "{}", outside);
    let spoof = secret_win::credential_target_name("evil.com.allinone.desktop", "x").unwrap_err();
    assert!(spoof.to_string().contains("命名空间"), "{}", spoof);
}

#[test]
fn credential_target_name_rejects_ambiguous_or_empty_parts() {
    // 空片段。
    assert!(secret_win::credential_target_name("", "a").is_err());
    assert!(secret_win::credential_target_name("com.allinone.desktop.a", "").is_err());
    // '/' 与 TargetName 分隔符冲突，会造成条目歧义。
    assert!(secret_win::credential_target_name("com.allinone.desktop.a", "b/c").is_err());
    assert!(secret_win::credential_target_name("com.allinone.desktop.a/b", "c").is_err());
    // NUL 会截断 C 字符串，可能读到别的条目。
    assert!(secret_win::credential_target_name("com.allinone.desktop.a", "b\0c").is_err());
    // 超过通用 TargetName 的 UTF-16 长度上限（32767 单元）。
    let long_account = "a".repeat(40_000);
    assert!(secret_win::credential_target_name("com.allinone.desktop.a", &long_account).is_err());
}

#[test]
fn credential_blob_size_limit_is_enforced() {
    // 二进制秘密按原样存取，只限制长度：2560 上限内合法，超限明确报错。
    assert!(secret_win::validate_secret_len(0).is_ok());
    assert!(secret_win::validate_secret_len(2560).is_ok());
    let error = secret_win::validate_secret_len(2561).unwrap_err();
    assert!(error.to_string().contains("2560"), "{}", error);
}

#[test]
fn win32_error_codes_map_to_actionable_hal_errors() {
    // HRESULT_FROM_WIN32 形式提取低 16 位错误码。
    assert_eq!(secret_win::win32_error_code(0x80070490u32 as i32), 1168);
    assert_eq!(secret_win::win32_error_code(0x80070005u32 as i32), 5);
    // 拒绝访问 → 权限错误（带能力名与指引）。
    match secret_win::cred_error("写入", 5) {
        HalError::PermissionDenied { capability, guidance } => {
            assert_eq!(capability, "credential_manager");
            assert!(guidance.contains("写入"));
        }
        other => panic!("ERROR_ACCESS_DENIED 应映射为 PermissionDenied：{}", other),
    }
    // 其余错误 → 原生调用失败，信息含动作与错误码但不含秘密。
    match secret_win::cred_error("读取", 1317) {
        HalError::NativeCallFailed(message) => {
            assert!(message.contains("读取"), "{}", message);
            assert!(message.contains("1317"), "{}", message);
        }
        other => panic!("一般 Win32 错误应映射为 NativeCallFailed：{}", other),
    }
}

#[test]
fn gif_write_rejects_empty_payload_without_touching_clipboard() {
    // 空内容在任何剪贴板操作之前被拒绝（与 macOS 口径一致）。
    let error = write_animated_gif_clipboard(&[]).unwrap_err();
    assert!(error.contains("GIF 内容为空"), "{}", error);
    let inner = clipboard_win::write_animated_gif_clipboard(&[]).unwrap_err();
    assert!(inner.to_string().contains("GIF 内容为空"), "{}", inner);
}

#[test]
fn alloc_moveable_round_trips_bytes_and_unlock_completes_normally() {
    // 覆盖 GlobalUnlock 的两条判定路径（实测语义）：
    // - 正常完成解锁：FALSE 且 last-error=0 → 必须视为成功，绝不允许误报失败；
    // - 真实失败：对已解锁句柄再次解锁，FALSE 且 last-error=ERROR_NOT_LOCKED(158) → Err。
    let payload: &[u8] = &[0x00, 0x41, 0xFF, 0x7F, 0x00, 0x42];
    let handle: HGLOBAL = clipboard_win::alloc_moveable(payload).unwrap();
    // SAFETY: handle 有效且未释放；重新加锁读回校验，再显式解锁、释放。
    unsafe {
        let view = GlobalLock(handle);
        assert!(!view.is_null());
        let size = GlobalSize(handle);
        assert!(size >= payload.len(), "分配尺寸 {} 不足载荷长度", size);
        let bytes = std::slice::from_raw_parts(view.cast::<u8>(), payload.len());
        assert_eq!(bytes, payload);
        // 首次解锁（锁计数归零）是「FALSE + last-error 0」的正常完成分支。
        assert!(clipboard_win::unlock_moveable(handle).is_ok());
        // 再次解锁是真实失败（ERROR_NOT_LOCKED），必须报错而不能当作正常完成。
        assert!(clipboard_win::unlock_moveable(handle).is_err());
        // GlobalFree 成功返回 NULL；windows-rs 的 Result 包装把 NULL 映射为 Err、
        // 把未释放句柄映射为 Ok，语义与直觉相反——按其真实语义断言（Err = 释放成功）。
        let free = GlobalFree(Some(handle));
        assert!(
            free.is_err(),
            "GlobalFree 释放成功（返回 NULL）在 windows-rs 包装中应表现为 Err：{:?}",
            free
        );
    }
}

#[test]
fn owner_window_creates_and_destroys_within_the_calling_thread() {
    // 消息窗口生命周期自管理：创建成功后销毁，不依赖主控台或任何外部窗口。
    let owner = clipboard_win::create_owner_window().unwrap();
    assert!(!owner.is_invalid());
    // SAFETY: 窗口由本测试线程创建且未销毁过。
    unsafe { DestroyWindow(owner).unwrap() };
}
