//! 外部窗口工具 IPC 适配。
use super::*;

impl DesktopHost {
    pub(super) fn dispatch_window_pin(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        match cmd {
            "winpin_sync_os_windows" => {
                let raw_windows = args
                    .get("windows")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new()));
                let parsed: Vec<ExternalWindowInfo> =
                    serde_json::from_value(raw_windows).unwrap_or_default();
                let fg_handle = args.get("foreground_handle").and_then(Value::as_u64);
                if !parsed.is_empty() {
                    self.mock_hal
                        .as_ref()
                        .ok_or_else(|| {
                            CoreError::Config(
                                "Mock window synchronization is unavailable in native mode".into(),
                            )
                        })?
                        .replace_external_windows(parsed, fg_handle);
                }
                let candidates = self.window_pin.list_candidate_windows()?;
                let fg = self.window_pin.current_foreground_window().ok();
                Ok(json!({
                    "candidates": candidates,
                    "foreground_handle": fg.map(|w| w.handle),
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_list_candidates" => {
                let candidates = self.window_pin.list_candidate_windows()?;
                let fg = self.window_pin.current_foreground_window().ok();
                Ok(json!({
                    "candidates": candidates,
                    "foreground_handle": fg.map(|w| w.handle),
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_list_all" => {
                let pinned = self.window_pin.list_pinned_windows();
                Ok(serde_json::to_value(pinned)?)
            }

            "winpin_set_foreground" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(1001);
                self.mock_hal
                    .as_ref()
                    .ok_or_else(|| {
                        CoreError::Config(
                            "Mock foreground selection is unavailable in native mode".into(),
                        )
                    })?
                    .set_foreground_handle(handle);
                let fg = self.window_pin.current_foreground_window()?;
                Ok(serde_json::to_value(fg)?)
            }

            "winpin_toggle_foreground" => {
                let (info, is_pinned) = self.window_pin.toggle_foreground_window()?;
                if !is_pinned {
                    let badge_label = format!("pin-badge-{}", info.handle);
                    let _ = ctx.window_pool.destroy_window(&badge_label);
                }
                Ok(json!({
                    "window": info,
                    "is_pinned": is_pinned,
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_toggle_handle" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(0);
                let (info, is_pinned) = self.window_pin.toggle_handle(handle)?;
                if !is_pinned {
                    let badge_label = format!("pin-badge-{}", handle);
                    let _ = ctx.window_pool.destroy_window(&badge_label);
                }
                Ok(json!({
                    "window": info,
                    "is_pinned": is_pinned,
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_pick_at_cursor" => {
                let x = args.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
                let y = args.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
                let (info, is_pinned) = self.window_pin.pick_at_cursor(x, y)?;
                // 拾取完成后自动销毁全屏准星选择窗口
                let _ = ctx.window_pool.destroy_window("window-pin-picker");
                Ok(json!({
                    "window": info,
                    "is_pinned": is_pinned,
                    "pinned": self.window_pin.list_pinned_windows(),
                    "active_window_count": ctx.window_pool.active_window_count(),
                }))
            }

            "winpin_get_snapshot" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(0);
                let data_url = self
                    .os_hal
                    .window_controller
                    .capture_window_data_url(handle)
                    .map_err(|error| CoreError::Internal(anyhow::anyhow!(error)))?;
                Ok(json!({ "handle": handle, "data_url": data_url }))
            }

            "winpin_activate" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(0);
                self.os_hal
                    .window_controller
                    .activate_external_window(handle)
                    .map_err(|error| CoreError::Internal(anyhow::anyhow!(error)))?;
                Ok(json!({ "handle": handle, "activated": true }))
            }

            "winpin_unpin" => {
                let handle = args.get("handle").and_then(Value::as_u64).unwrap_or(0);
                let info = self.window_pin.unpin_window(handle)?;
                let badge_label = format!("pin-badge-{}", handle);
                let _ = ctx.window_pool.destroy_window(&badge_label);
                Ok(json!({
                    "window": info,
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            "winpin_restore_all" => {
                let current_pinned = self.window_pin.list_pinned_windows();
                for item in current_pinned {
                    let badge_label = format!("pin-badge-{}", item.info.handle);
                    let _ = ctx.window_pool.destroy_window(&badge_label);
                }
                let restored = self.window_pin.restore_all_pinned_windows();
                Ok(json!({
                    "restored_count": restored,
                    "pinned": self.window_pin.list_pinned_windows(),
                }))
            }

            _ => Err(CoreError::Config(format!("Unknown IPC command: {}", cmd))),
        }
    }
}
