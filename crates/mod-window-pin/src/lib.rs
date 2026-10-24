//! `mod-window-pin` — Module 09: DeskPins-style External Window Topmost Utility.
//!
//! Features:
//! - **Dual Trigger Modes**:
//!   - `PrimaryMod+Shift+P` (`window_pin:toggle_foreground`): Toggles topmost (`HWND_TOPMOST` /
//!     macOS floating level) on the currently active foreground window.
//!   - `PrimaryMod+SecondaryMod+P` (`window_pin:pick_cursor`): Enters crosshair picker mode to
//!     pin any arbitrary third-party window at cursor coordinates `(x, y)`.
//! - **Guaranteed Cleanup on Shutdown**: Tracks all pinned external window handles in memory and
//!   automatically restores every pinned window to non-topmost (`HWND_NOTOPMOST`) inside
//!   `on_shutdown`, ensuring external apps are never stranded topmost when the daemon exits.

use app_core::{AppModule, CoreError, CoreResult, ModuleContext, ShortcutSpec};
use chrono::Utc;
use os_hal::{ExternalWindowInfo, MockOsHal, WindowController, WindowHandleId};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tracing::{info, warn};

/// Metadata for an external window currently pinned topmost by `mod-window-pin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedWindowEntry {
    pub info: ExternalWindowInfo,
    pub pinned_at: i64,
}

/// Module 09 implementation (`mod-window-pin`).
#[derive(Clone)]
pub struct WindowPinModule {
    window_controller: Arc<dyn WindowController>,
    pinned_windows: Arc<RwLock<HashMap<WindowHandleId, PinnedWindowEntry>>>,
}

impl Default for WindowPinModule {
    fn default() -> Self {
        Self::new(Arc::new(MockOsHal::new()))
    }
}

impl WindowPinModule {
    pub fn new(window_controller: Arc<dyn WindowController>) -> Self {
        Self {
            window_controller,
            pinned_windows: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Toggles topmost state for the current foreground window (`winpin_toggle_foreground` IPC).
    /// Returns `Ok((ExternalWindowInfo, is_now_pinned))`.
    pub fn toggle_foreground_window(&self) -> CoreResult<(ExternalWindowInfo, bool)> {
        let fg = self
            .window_controller
            .get_foreground_window()
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;
        self.toggle_handle(fg.handle)
    }

    /// Picks the external window at physical screen coordinates `(x, y)` and pins/toggles it
    /// (`winpin_pick_at_cursor` IPC).
    pub fn pick_at_cursor(&self, x: i32, y: i32) -> CoreResult<(ExternalWindowInfo, bool)> {
        let target = self
            .window_controller
            .get_window_at_point(x, y)
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;
        self.toggle_handle(target.handle)
    }

    /// Toggles topmost state for a specific external window `handle`.
    pub fn toggle_handle(&self, handle: WindowHandleId) -> CoreResult<(ExternalWindowInfo, bool)> {
        let currently_pinned = self.pinned_windows.read().contains_key(&handle);
        if currently_pinned {
            let info = self.unpin_window(handle)?;
            Ok((info, false))
        } else {
            let info = self.pin_window(handle)?;
            Ok((info, true))
        }
    }

    /// Explicitly pins an external window `handle` topmost.
    pub fn pin_window(&self, handle: WindowHandleId) -> CoreResult<ExternalWindowInfo> {
        self.window_controller
            .set_external_window_topmost(handle, true)
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;

        let updated_info = self
            .window_controller
            .get_window_info(handle)
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;

        let entry = PinnedWindowEntry {
            info: updated_info.clone(),
            pinned_at: Utc::now().timestamp_millis(),
        };
        self.pinned_windows.write().insert(handle, entry);
        info!(
            handle,
            title = %updated_info.title,
            "Pinned external window topmost"
        );
        Ok(updated_info)
    }

    /// Explicitly unpins an external window `handle` and restores its normal Z-order (`winpin_unpin` IPC).
    pub fn unpin_window(&self, handle: WindowHandleId) -> CoreResult<ExternalWindowInfo> {
        self.window_controller
            .set_external_window_topmost(handle, false)
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;

        self.pinned_windows.write().remove(&handle);
        let updated_info = self
            .window_controller
            .get_window_info(handle)
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;

        info!(
            handle,
            title = %updated_info.title,
            "Restored external window to non-topmost"
        );
        Ok(updated_info)
    }

    /// Lists all currently pinned external windows (`winpin_list_all` IPC).
    pub fn list_pinned_windows(&self) -> Vec<PinnedWindowEntry> {
        let mut entries: Vec<PinnedWindowEntry> =
            self.pinned_windows.read().values().cloned().collect();
        entries.sort_by_key(|e| e.pinned_at);
        entries
    }

    /// Lists all external windows known to the OS HAL (`winpin_list_candidates` IPC).
    pub fn list_candidate_windows(&self) -> CoreResult<Vec<ExternalWindowInfo>> {
        self.window_controller
            .list_external_windows()
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))
    }

    /// Queries the current foreground window from OS HAL.
    pub fn current_foreground_window(&self) -> CoreResult<ExternalWindowInfo> {
        self.window_controller
            .get_foreground_window()
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))
    }

    /// Unpins all currently tracked external windows (invoked automatically during `on_shutdown`).
    pub fn restore_all_pinned_windows(&self) -> usize {
        let handles: Vec<WindowHandleId> = self.pinned_windows.read().keys().copied().collect();
        let mut restored = 0;
        for handle in handles {
            match self
                .window_controller
                .set_external_window_topmost(handle, false)
            {
                Ok(()) => {
                    self.pinned_windows.write().remove(&handle);
                    restored += 1;
                }
                Err(err) => {
                    warn!(handle, error = %err, "Failed to unpin external window during shutdown cleanup");
                }
            }
        }
        restored
    }
}

impl AppModule for WindowPinModule {
    fn id(&self) -> &'static str {
        "window_pin"
    }

    fn name(&self) -> &'static str {
        "任意窗口置顶工具 (DeskPins)"
    }

    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![
            ShortcutSpec::new(
                "window_pin",
                "window_pin:toggle_foreground",
                "PrimaryMod+Shift+P",
                "置顶/取消置顶当前活动窗口",
            ),
            ShortcutSpec::new(
                "window_pin",
                "window_pin:pick_cursor",
                "PrimaryMod+SecondaryMod+P",
                "唤醒准星拾取器置顶任意外部窗口",
            ),
        ]
    }

    fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext) -> CoreResult<()> {
        if action_id == "window_pin:toggle_foreground" {
            self.toggle_foreground_window()?;
        } else if action_id == "window_pin:pick_cursor" {
            if ctx
                .window_pool
                .get_active_window("window-pin-picker")
                .is_some()
            {
                ctx.window_pool.destroy_window("window-pin-picker")?;
            } else {
                ctx.window_pool
                    .open_window(app_core::WindowSpec::window_pin_picker(1920.0, 1080.0))?;
            }
        }
        Ok(())
    }

    fn on_shutdown(&self, _ctx: &ModuleContext) -> CoreResult<()> {
        let restored = self.restore_all_pinned_windows();
        info!(
            restored_windows = restored,
            "WindowPinModule restored all pinned external windows before exit"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_core::{AppKernelBuilder, TargetOs};

    #[test]
    fn test_window_pin_toggle_and_auto_restore_on_shutdown() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mock_hal = MockOsHal::new();
        let winpin = WindowPinModule::new(Arc::new(mock_hal.clone()));

        let kernel = AppKernelBuilder::new(temp_dir.path())
            .with_in_memory_db(true)
            .with_target_os(TargetOs::Windows)
            .register_module(winpin.clone())
            .unwrap()
            .bootstrap()
            .unwrap();

        // Trigger PrimaryMod+Shift+P -> pins foreground window (handle 1001)
        kernel
            .trigger_action("window_pin:toggle_foreground")
            .unwrap();
        assert!(mock_hal.get_window_info(1001).unwrap().is_topmost);
        assert_eq!(winpin.list_pinned_windows().len(), 1);

        // Pin second window by cursor coordinates (250, 200 -> hits handle 1001 or 1002)
        winpin.pin_window(1002).unwrap();
        assert!(mock_hal.get_window_info(1002).unwrap().is_topmost);
        assert_eq!(winpin.list_pinned_windows().len(), 2);

        // Shut down kernel -> must automatically restore all external windows to non-topmost!
        kernel.shutdown().unwrap();
        assert!(!mock_hal.get_window_info(1001).unwrap().is_topmost);
        assert!(!mock_hal.get_window_info(1002).unwrap().is_topmost);
        assert_eq!(winpin.list_pinned_windows().len(), 0);
    }
}
