#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(target_os = "macos")]
pub use macos::NativeVaultObserver;
#[cfg(target_os = "windows")]
pub use windows::NativeVaultObserver;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub struct NativeVaultObserver;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl NativeVaultObserver {
    /// 其他平台没有经过验收的原生会话观察者，不能用空实现假装已经接入。
    pub fn start(_service: std::sync::Arc<crate::VaultService>) -> crate::VaultResult<Self> {
        Err(crate::VaultError::FeatureDisabled(
            "native power/session observer is not implemented on this platform",
        ))
    }
}
