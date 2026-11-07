//! 敏感数据相关的系统能力契约：剪贴板所有权与系统凭据库。
//!
//! 这两项能力不放入 `OsHalBundle`，由宿主按需单独注入给确实需要的模块（翻译凭据、密码本复制），
//! 避免其他模块获得不必要的敏感能力。
//!
//! # 本应用剪贴板监听器的配合要求
//!
//! 宿主的剪贴板历史监听器（`start_clipboard_listener`）收录内容前做两层过滤：
//! 1. 事件总线剪贴板屏蔽租约（`ClipboardSuppressionLease`）——秘密写入方在敏感内容停留于
//!    剪贴板的整个期间必须持有租约，否则监听器会把内容收录进历史；
//! 2. [`crate::clipboard_content_is_concealed`] 只读探测——内容带系统敏感/排除标记时跳过收录。
//!    该探测保守地把「CanIncludeInClipboardHistory / CanUploadToCloudClipboard」的存在（即使值为
//!    `1` 显式允许记录）也当作敏感处理，可能跳过少量可记录内容，但绝不会收录秘密。
//!
//! 两层过滤之后，写入本身仍以各平台的系统级标记为准（见 [`ClipboardOwnership`]）。

use crate::types::HalResult;
use serde::{Deserialize, Serialize};

/// 一次剪贴板写入的所有权凭证：只记录系统剪贴板序号，不保存写入内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardWriteToken {
    /// 写入完成后系统剪贴板的变更序号
    /// （macOS `NSPasteboard.changeCount`；Windows `GetClipboardSequenceNumber`）。
    /// 序号 `0` 表示系统未维护序号，不是可靠所有权凭证，写入路径会拒绝产生。
    pub sequence: i64,
}

/// 剪贴板所有权：写入敏感文本，并只在剪贴板仍为本次写入时才清除。
///
/// 「仍为本次内容」由系统剪贴板变更序号判定，而不是比较文本：用户之后复制了相同文本，
/// 序号也会变化，因此不会误清用户自己的内容。
pub trait ClipboardOwnership: Send + Sync + 'static {
    /// 写入文本并标记为敏感：
    /// - macOS 附加 `org.nspasteboard.ConcealedType`，遵守该约定的剪贴板管理器不会记录；
    /// - Windows 写入 `CF_UNICODETEXT` 并设置微软文档《Clipboard Formats》→ Cloud Clipboard
    ///   and Clipboard History Formats 小节的准确注册标记（注意注册名无空格）：
    ///   `ExcludeClipboardContentFromMonitorProcessing`（总控）与
    ///   `CanIncludeInClipboardHistory = 0` / `CanUploadToCloudClipboard = 0`（细粒度 DWORD），
    ///   Windows 剪贴板历史（Win+V）与云同步都不会记录；任一标记写入失败会回滚清空
    ///   并返回错误，绝不留下未隐藏的秘密。
    ///
    /// 调用方必须在敏感内容停留于剪贴板期间持有事件总线的剪贴板屏蔽租约，
    /// 否则本应用的剪贴板历史监听器会收录该内容（见模块文档）。
    ///
    /// Windows 的所有权凭证只在验证通过后产出：写入事务关闭后重新独占打开，
    /// 以本次写入的私有唯一标记 + 存活的 owner 窗口证明「当前内容仍是本次写入」，
    /// 证明成立才采集最终序号；被其他程序抢占（含复制相同文本）则安全放弃——
    /// 不产出凭证、不触碰剪贴板，绝不认领或清除他人内容。
    fn write_concealed_text(&self, text: &str) -> HalResult<ClipboardWriteToken>;

    /// 若剪贴板自写入后未被任何程序修改，则清空并返回 `true`；已被修改返回 `false` 且不触碰。
    ///
    /// 判定与清空之间无竞态：Windows 在独占打开剪贴板之后才比较系统序号；
    /// 序号 `0` 的凭证不被信任，保守不清除。
    fn clear_if_unchanged(&self, token: &ClipboardWriteToken) -> HalResult<bool>;
}

/// 系统凭据库（macOS 登录钥匙串通用密码项；Windows 凭据管理器通用凭据）。
///
/// 只存取本应用自己写入的条目，`service` 统一使用应用前缀（`com.allinone.desktop*`），
/// 不读取用户已有的其他凭据。
///
/// Windows 实现细节：`CRED_TYPE_GENERIC` + `CRED_PERSIST_LOCAL_MACHINE`（仅本机持久化，
/// 不随账户漫游），TargetName 为 `{service}/{account}`，秘密按二进制 blob 原样存取
/// （上限 2560 字节）。这是普通用户凭据存储——受当前用户账户保护，**不是** Windows Hello /
/// 生物识别保护，也不绑定受保护的设备密钥；设备密钥保护由密码本模块另行实现。
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
