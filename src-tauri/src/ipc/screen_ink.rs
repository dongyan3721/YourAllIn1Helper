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

                // 在纯测试环境（MockOsHal）下使用 mock 提供的单帧
                let mock_bg_frame = if self.mock_hal.is_some() {
                    Some(self.os_hal.screen_capturer.capture_monitor(None).unwrap_or(
                        os_hal::CapturedFrame {
                            width: 100,
                            height: 100,
                            scale_factor_milli: 1000,
                            rgba_bytes: vec![255u8; 100 * 100 * 4],
                        },
                    ))
                } else {
                    None
                };

                let result = self.screen_ink.composite_and_save(
                    ctx,
                    &ink_png_bytes,
                    copy_to_clipboard,
                    file_path,
                    mock_bg_frame,
                )?;
                Ok(serde_json::to_value(result)?)
            }

            "ink_get_config" => {
                let cfg: ScreenInkConfig = ctx.module_settings("screen_ink")?;
                Ok(serde_json::to_value(cfg)?)
            }

            _ => Err(CoreError::Config(format!(
                "Unknown screen ink command: {}",
                cmd
            ))),
        }
    }
}
