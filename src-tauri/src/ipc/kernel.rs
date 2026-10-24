//! 主程序状态、配置及功能开关的 IPC 适配。
use super::*;

impl DesktopHost {
    pub(super) fn dispatch_kernel(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        match cmd {
            "kernel_get_status" => {
                let active_windows = ctx.window_pool.list_active_windows();
                let shortcuts = ctx.shortcut_registry.list_all();
                let modules = self.kernel.registry().list_modules();
                let config = ctx.config.read().clone();
                let fg_window = self.window_pin.current_foreground_window().ok();
                let env_tools = vec![
                    ctx.env_probe.probe(ExternalToolKind::Python3, false),
                    ctx.env_probe.probe(ExternalToolKind::Osascript, false),
                    ctx.env_probe.probe(ExternalToolKind::Zsh, false),
                    ctx.env_probe.probe(ExternalToolKind::Node, false),
                    ctx.env_probe.probe(ExternalToolKind::Ffmpeg, false),
                ];

                Ok(json!({
                    "target_os": TargetOs::current(),
                    "active_window_count": active_windows.len(),
                    "active_windows": active_windows,
                    "modules": modules,
                    "module_catalog": self.module_catalog(),
                    "shortcuts": shortcuts,
                    "config": config,
                    "clipboard_suppressed": ctx.event_bus.is_clipboard_suppressed(),
                    "paste_trigger_count": self.os_hal.input_simulator.paste_trigger_count(),
                    "foreground_external_window": fg_window,
                    "env_tools": env_tools,
                }))
            }

            "kernel_trigger_shortcut" => {
                if let Some(action_id) = args.get("action_id").and_then(Value::as_str) {
                    self.kernel.trigger_action(action_id)?;
                } else if let Some(accel) = args.get("accelerator").and_then(Value::as_str) {
                    self.kernel.trigger_accelerator(accel)?;
                } else {
                    return Err(app_core::CoreError::Config(
                        "Missing 'action_id' or 'accelerator'".to_string(),
                    ));
                }
                Ok(json!({
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "active_windows": ctx.window_pool.list_active_windows(),
                }))
            }

            "kernel_open_dashboard" => {
                let route = args
                    .get("route")
                    .and_then(Value::as_str)
                    .unwrap_or("/overview");
                ctx.window_pool.open_window(WindowSpec::dashboard(route))?;
                Ok(json!({
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "active_windows": ctx.window_pool.list_active_windows(),
                }))
            }

            "kernel_destroy_window" => {
                let label = args.get("label").and_then(Value::as_str).unwrap_or("");
                let destroyed = ctx.window_pool.destroy_window(label)?;
                Ok(json!({
                    "destroyed": destroyed,
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "active_windows": ctx.window_pool.list_active_windows(),
                }))
            }

            "kernel_blur_window" => {
                let label = args.get("label").and_then(Value::as_str).unwrap_or("");
                let auto_destroyed = ctx.window_pool.on_window_blur(label)?;
                Ok(json!({
                    "auto_destroyed": auto_destroyed,
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "active_windows": ctx.window_pool.list_active_windows(),
                }))
            }

            "kernel_update_clipboard_config" => {
                let max_items = args
                    .get("max_history_items")
                    .and_then(Value::as_u64)
                    .unwrap_or(100) as usize;
                let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                self.kernel.update_config(|cfg| {
                    cfg.clipboard.max_history_items = max_items.clamp(1, 500);
                    cfg.clipboard.enabled = enabled;
                })?;
                Ok(serde_json::to_value(ctx.config.read().clone())?)
            }

            "kernel_shutdown" => {
                self.kernel.shutdown()?;
                Ok(json!({
                    "active_window_count": ctx.window_pool.active_window_count(),
                    "pinned_external_windows": self.window_pin.list_pinned_windows(),
                }))
            }

            "kernel_list_modules" => Ok(self.module_catalog()),
            "kernel_set_module_enabled" => {
                let id = args
                    .get("module_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CoreError::Config("Missing module_id".into()))?;
                let enabled = args
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| CoreError::Config("Missing enabled boolean".into()))?;
                self.kernel.set_module_enabled(id, enabled)?;
                Ok(self.module_catalog())
            }

            _ => Err(CoreError::Config(format!("Unknown IPC command: {}", cmd))),
        }
    }
}
