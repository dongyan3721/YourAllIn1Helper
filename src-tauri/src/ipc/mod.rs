//! IPC 路由与模块开关检查，防止直接调用已关闭功能。
use super::*;
mod clipboard;
mod kernel;
mod sticky_note;
mod window_pin;
impl DesktopHost {
    pub(crate) fn dispatch_inner(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        if cmd.starts_with("kernel_") {
            return self.dispatch_kernel(cmd, args);
        }
        let (module_id, handler): (&str, fn(&Self, &str, Value) -> CoreResult<Value>) =
            if cmd.starts_with("clipboard_") {
                ("clipboard", Self::dispatch_clipboard)
            } else if cmd.starts_with("sticky_") {
                ("sticky_note", Self::dispatch_sticky_note)
            } else if cmd.starts_with("winpin_") {
                ("window_pin", Self::dispatch_window_pin)
            } else {
                return Err(CoreError::Config(format!("Unknown IPC command: {}", cmd)));
            };
        self.kernel
            .registry()
            .with_enabled(module_id, || handler(self, cmd, args))
    }

    pub fn module_catalog(&self) -> Value {
        let registry = self.kernel.registry();
        Value::Array(app_core::MODULE_CATALOG.iter().map(|module| {
            let supported = module.id != "window_pin" || self.os_hal.window_controller.supports_external_topmost();
            json!({ "number": module.number, "id": module.id, "name": module.name,
                "available": registry.contains(module.id), "enabled": registry.is_enabled(module.id),
                "supported": supported,
                "unavailable_reason": if !registry.contains(module.id) { "待开发" } else if !supported { "当前系统不支持外部窗口真正置顶" } else { "" },
            })
        }).collect())
    }
}
