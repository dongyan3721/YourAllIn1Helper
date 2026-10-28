//! 桌面屏幕注释 IPC 适配。
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use mod_screen_ink::config::ScreenInkConfig;

impl DesktopHost {
    pub(super) fn dispatch_screen_ink(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        match cmd {
            "ink_get_status" => {
                let status = self.screen_ink.get_status();
                Ok(serde_json::to_value(status)?)
            }

            "ink_start_session" => {
                let status = self.screen_ink.start_session(ctx)?;
                Ok(serde_json::to_value(status)?)
            }

            "ink_exit_session" => {
                self.screen_ink.exit_session(ctx)?;
                Ok(json!({ "exited": true }))
            }

            "ink_set_passthrough" => {
                let passthrough = args
                    .get("passthrough")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let status = self.screen_ink.set_passthrough(ctx, passthrough)?;
                Ok(serde_json::to_value(status)?)
            }

            "ink_clear_canvas" => {
                self.screen_ink.clear_canvas(ctx)?;
                Ok(json!({ "cleared": true }))
            }

            "ink_save_snapshot" => {
                let raw_b64 = args
                    .get("ink_png_base64")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                // 剔除可能的 Data URL 前缀（如 "data:image/png;base64,"）
                let b64_clean = if let Some(idx) = raw_b64.find(',') {
                    &raw_b64[idx + 1..]
                } else {
                    raw_b64
                };
                let ink_png_bytes = STANDARD.decode(b64_clean).map_err(|e| {
                    CoreError::Internal(anyhow::anyhow!("Invalid base64 ink PNG: {}", e))
                })?;

                let copy_to_clipboard = args
                    .get("copy_to_clipboard")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);

                let file_path = args
                    .get("file_path")
                    .and_then(Value::as_str)
                    .map(String::from);

                // 仅通过注入的 ScreenCapturer 抓取真实底图，绝不在生产环境冒充伪成功
                let result = self.screen_ink.composite_and_save(
                    ctx,
                    &ink_png_bytes,
                    copy_to_clipboard,
                    file_path,
                )?;
                Ok(serde_json::to_value(result)?)
            }

            "ink_get_config" => {
                let mut cfg: ScreenInkConfig =
                    ctx.module_settings("screen_ink").unwrap_or_default();
                cfg.sanitize();
                Ok(serde_json::to_value(cfg)?)
            }

            "ink_apply_cursor" => {
                let shape = args.get("shape").and_then(Value::as_str).unwrap_or("");
                if !matches!(shape, "arrow" | "crosshair" | "cell" | "none") {
                    return Err(CoreError::Config(format!(
                        "Unknown screen ink cursor shape: {shape}"
                    )));
                }
                // 真正的光标切换在宿主主线程完成；这里只校验形状，避免领域模块依赖 AppKit。
                Ok(json!({ "shape": shape }))
            }

            "ink_update_settings" => {
                let mut cfg: ScreenInkConfig = serde_json::from_value(args).map_err(|e| {
                    CoreError::Config(format!("Invalid screen ink settings: {}", e))
                })?;
                cfg.sanitize();

                // 持久化至 AppConfig.modules.settings
                self.kernel.update_config(|config| {
                    config.modules.settings.insert(
                        "screen_ink".to_string(),
                        serde_json::to_value(&cfg).unwrap_or_default(),
                    );
                })?;

                Ok(serde_json::to_value(&cfg)?)
            }

            _ => Err(CoreError::Config(format!(
                "Unknown screen ink command: {}",
                cmd
            ))),
        }
    }
}
