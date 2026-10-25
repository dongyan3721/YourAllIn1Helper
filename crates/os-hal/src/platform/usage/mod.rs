//! 前台应用、键鼠空闲与锁屏/息屏检测；平台实现分文件维护。
//!
//! 生产适配器只返回真实系统状态；无法获取时返回 `Unsupported`/`NativeCallFailed`，
//! 不伪造 `false` 或测试快照。是否降级由业务模块决定并必须在能力列表里说明。

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use crate::platform::NativeOsHal;
use crate::{ActiveAppSnapshot, ActiveAppWatcher, HalResult};

#[cfg(target_os = "macos")]
impl NativeOsHal {
    pub(super) fn usage_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        self.usage_active_app_macos()
    }

    pub(super) fn usage_idle_seconds(&self) -> HalResult<u64> {
        self.usage_idle_seconds_macos()
    }

    pub(super) fn usage_screen_locked(&self) -> HalResult<bool> {
        self.usage_screen_locked_macos()
    }
}

#[cfg(target_os = "windows")]
impl NativeOsHal {
    pub(super) fn usage_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        self.usage_active_app_windows()
    }

    pub(super) fn usage_idle_seconds(&self) -> HalResult<u64> {
        self.usage_idle_seconds_windows()
    }

    pub(super) fn usage_screen_locked(&self) -> HalResult<bool> {
        self.usage_screen_locked_windows()
    }
}

impl ActiveAppWatcher for NativeOsHal {
    fn current_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            return self.usage_active_app();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(crate::HalError::Unsupported("native_active_app_detection"))
        }
    }

    fn system_idle_seconds(&self) -> HalResult<u64> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            return self.usage_idle_seconds();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(crate::HalError::Unsupported("native_idle_detection"))
        }
    }

    fn is_screen_locked(&self) -> HalResult<bool> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            return self.usage_screen_locked();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(crate::HalError::Unsupported("native_screen_lock_detection"))
        }
    }
}
