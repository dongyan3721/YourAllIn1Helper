//! 划词翻译 IPC 调度实现。
//!
//! 关键约束：
//! - 仅在模块开启（enabled）时被调用（由 with_enabled 门禁保护）；
//! - API Key 仅存入内存凭据库，绝不记录到日志、配置或控制台输出；
//! - 选区提取前保持第三方焦点，缺凭据或无权限如实向前端报告。

use super::*;
use mod_translator::config::TranslatorConfig;

impl DesktopHost {
    pub(super) fn dispatch_translator(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        match cmd {
            "translator_get_config" => {
                let cfg = self.translator.config();
                Ok(serde_json::to_value(cfg)?)
            }
            "translator_update_config" => {
                let new_cfg: TranslatorConfig = serde_json::from_value(args)?;
                self.translator.update_config(new_cfg)?;
                let new_cfg = self.translator.config();
                // 持久化非敏感设置到磁盘 config.json
                self.kernel.update_config(|config| {
                    if let Ok(val) = serde_json::to_value(&new_cfg) {
                        config
                            .modules
                            .settings
                            .insert("translator".to_string(), val);
                    }
                })?;
                Ok(json!({ "updated": true }))
            }
            "translator_take_selection" => {
                Ok(serde_json::to_value(self.translator.take_selection())?)
            }
            "translator_list_providers" => {
                let list = self.translator.service().list_providers_metadata();
                Ok(Value::Array(list))
            }
            "translator_set_api_key" => {
                let provider_id = args
                    .get("provider_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CoreError::Config("缺失 provider_id 参数".to_string()))?;
                let api_key = args.get("api_key").and_then(|v| v.as_str()).unwrap_or("");

                self.translator
                    .service()
                    .credentials()
                    .set_api_key(provider_id, api_key)?;
                Ok(json!({ "saved": true }))
            }
            "translator_extract_selection" => match self.translator.extract_selected_text() {
                Ok(Some(selection)) => Ok(json!({
                    "has_selection": true,
                    "text": selection.text,
                    "source": selection.source,
                    "app_name": selection.source_app_name,
                })),
                Ok(None) => Ok(json!({
                    "has_selection": false,
                    "text": null,
                })),
                Err(os_hal::HalError::PermissionDenied {
                    capability,
                    guidance,
                }) => Ok(json!({
                    "has_selection": false,
                    "permission_denied": true,
                    "capability": capability,
                    "guidance": guidance,
                })),
                Err(os_hal::HalError::Unsupported(reason)) => Ok(json!({
                    "has_selection": false,
                    "unsupported": true,
                    "reason": reason,
                })),
                Err(e) => Err(CoreError::Internal(anyhow::anyhow!("选区提取失败: {}", e))),
            },
            "translator_translate" => {
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CoreError::Config("缺失 text 待翻译文本".to_string()))?;
                let from = args.get("from").and_then(|v| v.as_str());
                let to = args.get("to").and_then(|v| v.as_str());
                let provider_id = args.get("provider_id").and_then(|v| v.as_str());
                let stream = args.get("stream").and_then(|v| v.as_bool()).unwrap_or(true);

                let req_id =
                    self.translator
                        .start_translation(text, from, to, provider_id, stream)?;

                Ok(json!({ "request_id": req_id }))
            }
            "translator_cancel" => {
                self.translator.cancel_current_translation();
                Ok(json!({ "cancelled": true }))
            }
            "translator_open_popover" => {
                let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(300.0);
                let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(200.0);
                let spec = WindowSpec::translate_popover(x, y);
                self.kernel.context().window_pool.open_window(spec)?;
                Ok(json!({ "opened": true }))
            }
            "translator_close_popover" => {
                self.translator.cancel_current_translation();
                self.kernel
                    .context()
                    .window_pool
                    .destroy_window("translate-popover")?;
                Ok(json!({ "closed": true }))
            }
            _ => Err(CoreError::Config(format!(
                "Unknown translator command: {}",
                cmd
            ))),
        }
    }
}
