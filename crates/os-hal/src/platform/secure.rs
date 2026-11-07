//! 剪贴板所有权与系统凭据库的原生实现；尚未接入的平台明确返回 `Unsupported`。
//!
//! Windows 子实现拆分在 `secure/` 目录：`clipboard_win`（剪贴板/GIF）、`secret_win`（凭据管理器）。
#[cfg(target_os = "windows")]
mod clipboard_win;
#[cfg(target_os = "windows")]
mod secret_win;

use super::NativeOsHal;
use crate::{
    secure::{ClipboardOwnership, ClipboardWriteToken, SecretStore},
    HalResult,
};
// macOS 实现与非 Windows 回退分支在此构造 HalError；Windows 分支的错误在子模块内构造。
#[cfg_attr(target_os = "windows", allow(unused_imports))]
use crate::HalError;

/// 把动态 GIF 放进系统剪贴板。确认保留前也可以复制。
#[cfg(target_os = "macos")]
pub fn write_animated_gif_clipboard(bytes: &[u8]) -> Result<(), String> {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSData, NSString};

    if bytes.is_empty() {
        return Err("GIF 内容为空".to_string());
    }
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    let data = unsafe { NSData::dataWithBytes_length(bytes.as_ptr().cast(), bytes.len()) };
    let gif_type = NSString::from_str("com.compuserve.gif");
    if !pasteboard.setData_forType(Some(&data), &gif_type) {
        return Err("NSPasteboard 写入 GIF 失败".to_string());
    }
    let _ = pasteboard.setData_forType(Some(&data), &NSString::from_str("public.gif"));
    Ok(())
}

/// Windows：注册 `image/gif`（主）与 `GIF`（辅）格式写入 GIF 字节。
#[cfg(target_os = "windows")]
pub fn write_animated_gif_clipboard(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("GIF 内容为空".to_string());
    }
    clipboard_win::write_animated_gif_clipboard(bytes).map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn write_animated_gif_clipboard(_bytes: &[u8]) -> Result<(), String> {
    Err("当前系统还不能把动态 GIF 写入剪贴板".to_string())
}

/// macOS 剪贴板管理器约定的敏感类型标记（nspasteboard.org）。
#[cfg(target_os = "macos")]
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

/// 只读探测当前系统剪贴板内容是否带敏感/排除标记（Windows 排除历史标记 / macOS ConcealedType）。
///
/// 供本应用剪贴板监听器在收录前过滤秘密；`false` 不代表内容一定不敏感，
/// 秘密写入方仍须持有事件总线的剪贴板屏蔽租约（见 `os_hal::secure` 契约文档）。
/// 不打开、不修改剪贴板。
pub fn clipboard_content_is_concealed() -> bool {
    #[cfg(target_os = "windows")]
    {
        clipboard_win::clipboard_content_is_concealed()
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSPasteboard;
        use objc2_foundation::NSString;
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard
            .stringForType(&NSString::from_str(CONCEALED_TYPE))
            .is_some()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

impl ClipboardOwnership for NativeOsHal {
    #[cfg(target_os = "macos")]
    fn write_concealed_text(&self, text: &str) -> HalResult<ClipboardWriteToken> {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
        use objc2_foundation::NSString;

        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        // SAFETY: NSPasteboardTypeString 是 AppKit 导出的常量字符串，进程生命周期内有效。
        let string_type = unsafe { NSPasteboardTypeString };
        if !pasteboard.setString_forType(&NSString::from_str(text), string_type) {
            return Err(HalError::NativeCallFailed(
                "NSPasteboard 写入文本失败".to_string(),
            ));
        }
        // 敏感标记写入失败不影响文本本身，但必须报告，避免调用方误以为已对其他管理器隐藏。
        if !pasteboard
            .setString_forType(&NSString::from_str(""), &NSString::from_str(CONCEALED_TYPE))
        {
            pasteboard.clearContents();
            return Err(HalError::NativeCallFailed(
                "NSPasteboard 写入敏感类型标记失败，已清空剪贴板".to_string(),
            ));
        }
        Ok(ClipboardWriteToken {
            sequence: pasteboard.changeCount() as i64,
        })
    }

    #[cfg(target_os = "macos")]
    fn clear_if_unchanged(&self, token: &ClipboardWriteToken) -> HalResult<bool> {
        use objc2_app_kit::NSPasteboard;
        let pasteboard = NSPasteboard::generalPasteboard();
        if pasteboard.changeCount() as i64 != token.sequence {
            return Ok(false);
        }
        pasteboard.clearContents();
        Ok(true)
    }

    #[cfg(target_os = "windows")]
    fn write_concealed_text(&self, text: &str) -> HalResult<ClipboardWriteToken> {
        clipboard_win::write_concealed_text(text)
    }

    #[cfg(target_os = "windows")]
    fn clear_if_unchanged(&self, token: &ClipboardWriteToken) -> HalResult<bool> {
        clipboard_win::clear_if_unchanged(token)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn write_concealed_text(&self, _text: &str) -> HalResult<ClipboardWriteToken> {
        Err(HalError::Unsupported("clipboard_ownership"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn clear_if_unchanged(&self, _token: &ClipboardWriteToken) -> HalResult<bool> {
        Err(HalError::Unsupported("clipboard_ownership"))
    }
}

/// 钥匙串「条目不存在」错误码（errSecItemNotFound）。
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

impl SecretStore for NativeOsHal {
    fn is_available(&self) -> bool {
        cfg!(any(target_os = "macos", target_os = "windows"))
    }

    #[cfg(target_os = "macos")]
    fn get_secret(&self, service: &str, account: &str) -> HalResult<Option<Vec<u8>>> {
        match security_framework::passwords::get_generic_password(service, account) {
            Ok(secret) => Ok(Some(secret)),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(keychain_error("读取", error)),
        }
    }

    #[cfg(target_os = "macos")]
    fn set_secret(&self, service: &str, account: &str, secret: &[u8]) -> HalResult<()> {
        security_framework::passwords::set_generic_password(service, account, secret)
            .map_err(|error| keychain_error("写入", error))
    }

    #[cfg(target_os = "macos")]
    fn delete_secret(&self, service: &str, account: &str) -> HalResult<()> {
        match security_framework::passwords::delete_generic_password(service, account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(error) => Err(keychain_error("删除", error)),
        }
    }

    #[cfg(target_os = "windows")]
    fn get_secret(&self, service: &str, account: &str) -> HalResult<Option<Vec<u8>>> {
        secret_win::get_secret(service, account)
    }

    #[cfg(target_os = "windows")]
    fn set_secret(&self, service: &str, account: &str, secret: &[u8]) -> HalResult<()> {
        secret_win::set_secret(service, account, secret)
    }

    #[cfg(target_os = "windows")]
    fn delete_secret(&self, service: &str, account: &str) -> HalResult<()> {
        secret_win::delete_secret(service, account)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn get_secret(&self, _service: &str, _account: &str) -> HalResult<Option<Vec<u8>>> {
        Err(HalError::Unsupported("system_secret_store"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn set_secret(&self, _service: &str, _account: &str, _secret: &[u8]) -> HalResult<()> {
        Err(HalError::Unsupported("system_secret_store"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn delete_secret(&self, _service: &str, _account: &str) -> HalResult<()> {
        Err(HalError::Unsupported("system_secret_store"))
    }
}

/// 钥匙串错误：用户拒绝访问单独归类为权限错误，其余为原生调用失败；错误信息不含密钥内容。
#[cfg(target_os = "macos")]
fn keychain_error(action: &str, error: security_framework::base::Error) -> HalError {
    // errSecAuthFailed / errSecUserCanceled / errSecInteractionNotAllowed
    match error.code() {
        -25293 | -128 | -25308 => HalError::PermissionDenied {
            capability: "keychain",
            guidance: format!(
                "钥匙串{}被拒绝或当前无法交互（{}）。请在钥匙串访问弹窗中允许本应用，或解锁登录钥匙串后重试",
                action, error
            ),
        },
        _ => HalError::NativeCallFailed(format!("钥匙串{}失败：{}", action, error)),
    }
}

/// Windows 子实现的纯逻辑回归单测（不触碰真实剪贴板与凭据库）。
#[cfg(all(test, target_os = "windows"))]
#[path = "../../tests/unit/secure_windows.rs"]
mod windows_tests;
