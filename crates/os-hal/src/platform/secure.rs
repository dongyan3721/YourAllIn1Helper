//! 剪贴板所有权与系统凭据库的原生实现；尚未接入的平台明确返回 `Unsupported`。
use super::NativeOsHal;
use crate::{
    secure::{ClipboardOwnership, ClipboardWriteToken, SecretStore},
    HalError, HalResult,
};

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

#[cfg(not(target_os = "macos"))]
pub fn write_animated_gif_clipboard(_bytes: &[u8]) -> Result<(), String> {
    Err("当前系统还不能把动态 GIF 写入剪贴板".to_string())
}

/// macOS 剪贴板管理器约定的敏感类型标记（nspasteboard.org）。
#[cfg(target_os = "macos")]
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

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

    #[cfg(not(target_os = "macos"))]
    fn write_concealed_text(&self, _text: &str) -> HalResult<ClipboardWriteToken> {
        Err(HalError::Unsupported("clipboard_ownership"))
    }

    #[cfg(not(target_os = "macos"))]
    fn clear_if_unchanged(&self, _token: &ClipboardWriteToken) -> HalResult<bool> {
        Err(HalError::Unsupported("clipboard_ownership"))
    }
}

/// 钥匙串「条目不存在」错误码（errSecItemNotFound）。
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

impl SecretStore for NativeOsHal {
    fn is_available(&self) -> bool {
        cfg!(target_os = "macos")
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

    #[cfg(not(target_os = "macos"))]
    fn get_secret(&self, _service: &str, _account: &str) -> HalResult<Option<Vec<u8>>> {
        Err(HalError::Unsupported("system_secret_store"))
    }

    #[cfg(not(target_os = "macos"))]
    fn set_secret(&self, _service: &str, _account: &str, _secret: &[u8]) -> HalResult<()> {
        Err(HalError::Unsupported("system_secret_store"))
    }

    #[cfg(not(target_os = "macos"))]
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
