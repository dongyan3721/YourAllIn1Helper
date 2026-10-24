//! Tauri 原生全局快捷键同步器：只迁移单个脚本动作，失败时尽量保留旧注册。
use app_core::RegisteredShortcut;
use mod_script_runner::hotkey::ScriptHotkeySync;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::DesktopHost;

pub struct TauriScriptHotkeySync {
    app: AppHandle,
}

impl TauriScriptHotkeySync {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn register(&self, action_id: &str, accelerator: &str) -> Result<(), String> {
        let action = action_id.to_string();
        self.app
            .global_shortcut()
            .on_shortcut(accelerator, move |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    if let Some(host) = app.try_state::<Arc<DesktopHost>>() {
                        if let Err(error) = host.kernel().trigger_action(&action) {
                            tracing::error!(action = %action, %error, "Script hotkey action failed");
                        }
                    }
                    let _ = app.emit("kernel://updated", Value::Null);
                }
            })
            .map_err(|error| format!("注册 {accelerator} 失败：{error}"))
    }

    fn unregister(&self, accelerator: &str) -> Result<(), String> {
        self.app
            .global_shortcut()
            .unregister(accelerator)
            .map_err(|error| format!("注销 {accelerator} 失败：{error}"))
    }
}

impl ScriptHotkeySync for TauriScriptHotkeySync {
    fn sync(
        &self,
        action_id: &str,
        previous: Option<&RegisteredShortcut>,
        desired: Option<&RegisteredShortcut>,
    ) -> Result<(), String> {
        if let (Some(previous), Some(desired)) = (previous, desired) {
            if previous.physical_accelerator == desired.physical_accelerator {
                return Ok(());
            }
        }

        match (previous, desired) {
            (None, None) => Ok(()),
            (None, Some(desired)) => self.register(action_id, &desired.physical_accelerator),
            (Some(previous), None) => self.unregister(&previous.physical_accelerator),
            (Some(previous), Some(desired)) => {
                // 先注册新快捷键：失败时旧快捷键完全不受影响。
                self.register(action_id, &desired.physical_accelerator)?;
                if let Err(error) = self.unregister(&previous.physical_accelerator) {
                    // 旧快捷键仍可能有效；撤销刚注册的新快捷键，避免双重绑定。
                    if let Err(rollback_error) = self.unregister(&desired.physical_accelerator) {
                        return Err(format!(
                            "{error}；撤销新快捷键也失败，旧快捷键仍保留：{rollback_error}"
                        ));
                    }
                    return Err(format!("{error}；已撤销新快捷键并保留旧快捷键"));
                }
                Ok(())
            }
        }
    }
}
