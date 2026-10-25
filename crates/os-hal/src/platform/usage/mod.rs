//! 前台应用、键鼠空闲、锁屏/息屏/睡眠与音频活动检测；平台实现分文件维护。
//!
//! 生产适配器只返回真实系统状态；无法获取时返回 `Unsupported`/`NativeCallFailed`，
//! 不伪造 `false` 或测试快照。是否降级由业务模块决定并必须在能力列表里说明。
//!
//! macOS 的锁屏/睡眠同时使用严格轮询与系统通知：任一来源报告暂停即暂停；
//! 唤醒后必须由通知或轮询明确确认已解锁才恢复，未知状态保持暂停。

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_events;
#[cfg(target_os = "windows")]
mod windows;

use crate::platform::NativeOsHal;
use crate::{ActiveAppSnapshot, ActiveAppWatcher, HalResult};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[cfg(target_os = "macos")]
impl NativeOsHal {
    pub(super) fn usage_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        self.usage_active_app_macos()
    }

    pub(super) fn usage_idle_seconds(&self) -> HalResult<u64> {
        self.usage_idle_seconds_macos()
    }

    pub(super) fn usage_screen_locked(&self) -> HalResult<bool> {
        let polled = self.usage_screen_locked_macos();
        self.usage_events.merge_screen_locked(polled)
    }

    pub(super) fn usage_audio_active(&self) -> HalResult<bool> {
        self.usage_audio_active_macos()
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
        let polled = self.usage_screen_locked_windows();
        self.usage_events.merge_screen_locked(polled)
    }

    pub(super) fn usage_audio_active(&self) -> HalResult<bool> {
        // Windows 尚未实现真实音频活动探测，明确返回 Unsupported。
        Err(crate::HalError::Unsupported(
            "windows_default_output_audio_activity",
        ))
    }
}

/// 系统活动事件的三种状态；`Unknown` 表示尚未收到事件且轮询也不可用。
const ACTIVITY_OFF: u8 = 1;
const ACTIVITY_ON: u8 = 2;

/// 平台无关的事件类型，供 macOS 观察者与测试共用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UsageActivityEvent {
    WillSleep,
    DidWake,
    ScreensDidSleep,
    ScreensDidWake,
    ScreenLocked,
    ScreenUnlocked,
}

/// 由系统通知维护的锁屏/睡眠状态。
///
/// 只保存最近一次已知状态；没有收到任何事件时保持 `Unknown`，
/// 这样调用方可以区分“明确清醒”和“未知”，不会把未知当成活跃。
#[derive(Debug, Default)]
pub(crate) struct UsageActivityState {
    sleeping: AtomicU8,
    screens_asleep: AtomicU8,
    locked: AtomicU8,
}

impl UsageActivityState {
    pub(crate) fn apply(&self, event: UsageActivityEvent) {
        let (target, value) = match event {
            UsageActivityEvent::WillSleep => (&self.sleeping, true),
            UsageActivityEvent::DidWake => (&self.sleeping, false),
            UsageActivityEvent::ScreensDidSleep => (&self.screens_asleep, true),
            UsageActivityEvent::ScreensDidWake => (&self.screens_asleep, false),
            UsageActivityEvent::ScreenLocked => (&self.locked, true),
            UsageActivityEvent::ScreenUnlocked => (&self.locked, false),
        };
        target.store(
            if value { ACTIVITY_ON } else { ACTIVITY_OFF },
            Ordering::SeqCst,
        );
    }

    fn flag(value: &AtomicU8) -> Option<bool> {
        match value.load(Ordering::SeqCst) {
            ACTIVITY_ON => Some(true),
            ACTIVITY_OFF => Some(false),
            _ => None,
        }
    }

    /// 合并系统事件与严格轮询结果。
    ///
    /// - 任一来源报告锁定/睡眠：返回 `Ok(true)`。
    /// - 通知明确报告“已唤醒且已解锁”：即使轮询暂时失败也恢复。
    /// - 轮询成功时以其结果为准（事件未知时仍可正常判断）。
    /// - 轮询失败且事件未完整确认清醒：返回错误，调用方保持暂停。
    pub(crate) fn merge_screen_locked(&self, polled: HalResult<bool>) -> HalResult<bool> {
        let sleeping = Self::flag(&self.sleeping);
        let screens_asleep = Self::flag(&self.screens_asleep);
        let locked = Self::flag(&self.locked);

        let asleep = match (sleeping, screens_asleep) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        };

        if asleep == Some(true) || locked == Some(true) {
            return Ok(true);
        }

        match polled {
            Ok(true) => Ok(true),
            Ok(false) => Ok(false),
            Err(error) => {
                // 解锁且至少一个唤醒来源确认屏幕已醒，才允许在轮询失败时恢复。
                // 只有 unlocked 通知、没有任何唤醒来源时仍视为未知并暂停。
                let confirmed_awake = locked == Some(false)
                    && (sleeping == Some(false) || screens_asleep == Some(false));
                if confirmed_awake {
                    Ok(false)
                } else {
                    Err(error)
                }
            }
        }
    }
}

/// 平台事件监视器的统一包装；非 macOS 平台保持 Unsupported。
#[derive(Clone)]
pub(super) struct NativeUsageEvents {
    state: Arc<UsageActivityState>,
    #[cfg(target_os = "macos")]
    monitor: Arc<macos_events::MacUsageEventMonitor>,
    #[cfg(not(target_os = "macos"))]
    monitor: (),
}

impl Default for NativeUsageEvents {
    fn default() -> Self {
        let state = Arc::new(UsageActivityState::default());
        Self {
            #[cfg(target_os = "macos")]
            monitor: Arc::new(macos_events::MacUsageEventMonitor::new(Arc::clone(&state))),
            #[cfg(not(target_os = "macos"))]
            monitor: (),
            state,
        }
    }
}

impl NativeUsageEvents {
    pub(super) fn start(&self) -> HalResult<()> {
        #[cfg(target_os = "macos")]
        {
            self.monitor.start()
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = &self.monitor;
            Err(crate::HalError::Unsupported("usage_event_monitor"))
        }
    }

    pub(super) fn stop(&self) {
        #[cfg(target_os = "macos")]
        self.monitor.stop();
    }

    pub(super) fn merge_screen_locked(&self, polled: HalResult<bool>) -> HalResult<bool> {
        self.state.merge_screen_locked(polled)
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

    fn default_output_audio_active(&self) -> HalResult<bool> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            return self.usage_audio_active();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(crate::HalError::Unsupported(
                "native_default_output_audio_activity",
            ))
        }
    }

    fn start_usage_event_monitor(&self) -> HalResult<()> {
        self.usage_events.start()
    }

    fn stop_usage_event_monitor(&self) {
        self.usage_events.stop();
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/usage_events.rs"]
mod tests;
