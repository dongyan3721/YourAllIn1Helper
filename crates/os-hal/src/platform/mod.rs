//! 原生适配器装配入口；业务模块只依赖 Trait，不依赖这里的具体实现。
mod biometry;
mod input;
mod ocr;
mod regions;
#[cfg(target_os = "macos")]
mod regions_ax;
mod screen;
#[cfg(target_os = "macos")]
mod screen_permission;
mod secure;
mod selection;
mod services;
mod usage;
mod windows;
#[cfg(target_os = "windows")]
mod winocr;

pub use biometry::{touch_id_probe, TouchIdProbe};
pub use secure::{
    clipboard_content_is_concealed, write_animated_gif_clipboard, write_rgba_clipboard,
};

use crate::{HalError, OsHalBundle, WindowHandleId};
use std::sync::{atomic::AtomicUsize, Arc};

pub(super) fn native_error(error: impl std::fmt::Display) -> HalError {
    HalError::NativeCallFailed(error.to_string())
}

/// Production adapter. Deterministic state belongs exclusively to `MockOsHal`.
#[derive(Clone, Default)]
pub struct NativeOsHal {
    #[cfg(target_os = "macos")]
    paste_target: Arc<std::sync::Mutex<Option<i32>>>,
    #[cfg(target_os = "windows")]
    paste_target: Arc<std::sync::Mutex<Option<(isize, u32)>>>,
    paste_triggers: Arc<AtomicUsize>,
    /// 已登记的原生捕获排除窗口。Windows 为 64 位 HWND，macOS 不使用持久排除。
    capture_exclusion: Arc<parking_lot::Mutex<Option<WindowHandleId>>>,
    /// 锁屏/息屏/睡眠事件观察者；平台无关包装，业务模块只看到 trait。
    usage_events: usage::NativeUsageEvents,
}
impl NativeOsHal {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn into_bundle(self) -> OsHalBundle {
        OsHalBundle::from_adapter(self)
    }
}
