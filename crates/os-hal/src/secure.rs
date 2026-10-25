//! 敏感数据相关的系统能力契约：剪贴板所有权与系统凭据库。
//!
//! 这两项能力不放入 `OsHalBundle`，由宿主按需单独注入给确实需要的模块（翻译凭据、密码本复制），
//! 避免其他模块获得不必要的敏感能力。

use crate::types::HalResult;
use serde::{Deserialize, Serialize};

/// 一次剪贴板写入的所有权凭证：只记录系统剪贴板序号，不保存写入内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardWriteToken {
    /// 写入完成后系统剪贴板的变更序号（macOS `NSPasteboard.changeCount`）。
    pub sequence: i64,
}

/// 剪贴板所有权：写入敏感文本，并只在剪贴板仍为本次写入时才清除。
///
/// 「仍为本次内容」由系统剪贴板变更序号判定，而不是比较文本：用户之后复制了相同文本，
/// 序号也会变化，因此不会误清用户自己的内容。
pub trait ClipboardOwnership: Send + Sync + 'static {
    /// 写入文本并标记为敏感（macOS 附加 `org.nspasteboard.ConcealedType`，遵守该约定的
    /// 剪贴板管理器不会记录）；返回写入后的所有权凭证。
    fn write_concealed_text(&self, text: &str) -> HalResult<ClipboardWriteToken>;

    /// 若剪贴板自写入后未被任何程序修改，则清空并返回 `true`；已被修改返回 `false` 且不触碰。
    fn clear_if_unchanged(&self, token: &ClipboardWriteToken) -> HalResult<bool>;
}

/// 系统凭据库（macOS 登录钥匙串通用密码项；Windows 凭据管理器尚未接入）。
///
/// 只存取本应用自己写入的条目，`service` 统一使用应用前缀，不读取用户已有的其他凭据。
pub trait SecretStore: Send + Sync + 'static {
    /// 是否已接入真实系统凭据库。返回 `false` 时调用方必须如实告知「仅本次会话有效」等限制。
    fn is_available(&self) -> bool;

    /// 读取条目；不存在返回 `Ok(None)`。
    fn get_secret(&self, service: &str, account: &str) -> HalResult<Option<Vec<u8>>>;

    /// 写入或覆盖条目。
    fn set_secret(&self, service: &str, account: &str, secret: &[u8]) -> HalResult<()>;

    /// 删除条目；不存在视为成功。
    fn delete_secret(&self, service: &str, account: &str) -> HalResult<()>;
}
