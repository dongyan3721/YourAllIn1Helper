//! `mod-window-pin` —— 子功能 09：任意窗口置顶小工具 (DeskPins)
//!
//! 核心设计与特性：
//! - **双触发模式**：
//!   - `PrimaryMod+Shift+P` (`window_pin:toggle_foreground`)：直接置顶/取消置顶当前活动的前台窗口；
//!   - `PrimaryMod+SecondaryMod+P` (`window_pin:pick_cursor`)：唤醒全屏准星拾取层，点击任意第三方外部窗口置顶；
//! - **程序退出兜底还原保证**：内存中严密跟踪所有处于置顶状态的外部窗口句柄，在微内核守护进程退出 (`on_shutdown`) 或异常崩溃时，
//!   自动遍历并恢复所有外部窗口为普通非置顶层级（Windows `HWND_NOTOPMOST` / macOS 销毁置顶镜像），防止外部窗口残留置顶导致系统桌面失控。

use app_core::{AppModule, CoreError, CoreResult, ModuleContext, ShortcutSpec};
use chrono::Utc;
use os_hal::{ExternalWindowInfo, MockOsHal, WindowController, WindowHandleId};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tracing::{info, warn};

/// 记录被本工具置顶的外部窗口元数据
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedWindowEntry {
    /// 外部窗口静态快照信息（包含句柄、标题、进程名、PID、坐标尺寸等）
    pub info: ExternalWindowInfo,
    /// 置顶时间戳（毫秒）
    pub pinned_at: i64,
}

/// 窗口置顶模块实现
#[derive(Clone)]
pub struct WindowPinModule {
    /// 跨平台窗口控制器（封装底层 Win32 API 或 macOS 抽象接口）
    window_controller: Arc<dyn WindowController>,
    /// 内存中维护的已置顶外部窗口句柄映射表
    pinned_windows: Arc<RwLock<HashMap<WindowHandleId, PinnedWindowEntry>>>,
}

impl Default for WindowPinModule {
    fn default() -> Self {
        Self::new(Arc::new(MockOsHal::new()))
    }
}

impl WindowPinModule {
    /// 创建窗口置顶模块实例
    pub fn new(window_controller: Arc<dyn WindowController>) -> Self {
        Self {
            window_controller,
            pinned_windows: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 切换当前前台活动窗口的置顶状态（`winpin_toggle_foreground` IPC）
    /// 返回 `Ok((ExternalWindowInfo, is_now_pinned))`
    pub fn toggle_foreground_window(&self) -> CoreResult<(ExternalWindowInfo, bool)> {
        let fg = self
            .window_controller
            .get_foreground_window()
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;
        self.toggle_handle(fg.handle)
    }

    /// 拾取物理屏幕坐标 `(x, y)` 处的外部窗口并切换其置顶状态（`winpin_pick_at_cursor` IPC）
    pub fn pick_at_cursor(&self, x: i32, y: i32) -> CoreResult<(ExternalWindowInfo, bool)> {
        let target = self
            .window_controller
            .get_window_at_point(x, y)
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;
        self.toggle_handle(target.handle)
    }

    /// 切换指定外部窗口句柄 `handle` 的置顶状态
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

    /// 将指定外部窗口设为置顶最前
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
            "成功将外部窗口设为置顶最前"
        );
        Ok(updated_info)
    }

    /// 取消指定外部窗口的置顶最前状态，恢复正常 Z 轴层级（`winpin_unpin` IPC）
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
            "已恢复外部窗口为普通非置顶层级"
        );
        Ok(updated_info)
    }

    /// 列出所有当前被本工具置顶的外部窗口列表（`winpin_list_all` IPC）
    pub fn list_pinned_windows(&self) -> Vec<PinnedWindowEntry> {
        let mut entries: Vec<PinnedWindowEntry> =
            self.pinned_windows.read().values().cloned().collect();
        entries.sort_by_key(|e| e.pinned_at);
        entries
    }

    /// 列出操作系统当前所有可见的外部窗口候选列表（`winpin_list_candidates` IPC）
    pub fn list_candidate_windows(&self) -> CoreResult<Vec<ExternalWindowInfo>> {
        self.window_controller
            .list_external_windows()
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))
    }

    /// 查询当前获得焦点的前台外部窗口
    pub fn current_foreground_window(&self) -> CoreResult<ExternalWindowInfo> {
        self.window_controller
            .get_foreground_window()
            .map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))
    }

    /// 触发所有已置顶外部窗口的置顶提升动作（在用户切换前台应用时无感触发，保持置顶）
    pub fn raise_all_pinned_windows(&self) -> usize {
        let handles: Vec<WindowHandleId> = self.pinned_windows.read().keys().copied().collect();
        let mut count = 0;
        for handle in handles {
            if self.window_controller.set_external_window_topmost(handle, true).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// 恢复所有已置顶的外部窗口（在守护进程退出 `on_shutdown` 时自动调用）
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
                    warn!(handle, error = %err, "退出还原外部窗口置顶状态失败");
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
            "WindowPinModule 在退出前已安全还原所有置顶外部窗口"
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

        // 触发快捷键 PrimaryMod+Shift+P -> 置顶前台窗口 (句柄 1001)
        kernel
            .trigger_action("window_pin:toggle_foreground")
            .unwrap();
        assert!(mock_hal.get_window_info(1001).unwrap().is_topmost);
        assert_eq!(winpin.list_pinned_windows().len(), 1);

        // 置顶第二个窗口 (句柄 1002)
        winpin.pin_window(1002).unwrap();
        assert!(mock_hal.get_window_info(1002).unwrap().is_topmost);
        assert_eq!(winpin.list_pinned_windows().len(), 2);

        // 守护进程退出关闭 -> 必须自动将所有外部窗口还原为普通非置顶状态！
        kernel.shutdown().unwrap();
        assert!(!mock_hal.get_window_info(1001).unwrap().is_topmost);
        assert!(!mock_hal.get_window_info(1002).unwrap().is_topmost);
        assert_eq!(winpin.list_pinned_windows().len(), 0);
    }
}
