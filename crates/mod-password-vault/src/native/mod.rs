#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::NativeVaultObserver;

#[cfg(not(target_os = "macos"))]
pub struct NativeVaultObserver;

#[cfg(not(target_os = "macos"))]
impl NativeVaultObserver {
    /// Windows 原生电源/会话通知尚未通过 M1 验收，不以空观察者冒充已接入。
    pub fn start(_service: std::sync::Arc<crate::VaultService>) -> crate::VaultResult<Self> {
        Err(crate::VaultError::FeatureDisabled(
            "native power/session observer is not implemented on this platform",
        ))
    }
}
