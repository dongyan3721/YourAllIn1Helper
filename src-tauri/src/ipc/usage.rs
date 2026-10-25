//! 应用使用时长 IPC 适配：仪表盘报表、应用分类/忽略名单与模块配置。
use super::*;
use mod_app_usage::config::AppUsageConfig;

impl DesktopHost {
    pub(super) fn dispatch_usage(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        match cmd {
            "usage_get_status" => Ok(serde_json::to_value(self.app_usage.status())?),

            "usage_get_overview" => {
                let (default_start, default_end) = self.app_usage.default_range();
                let start = args
                    .get("start_date")
                    .and_then(Value::as_str)
                    .unwrap_or(&default_start);
                let end = args
                    .get("end_date")
                    .and_then(Value::as_str)
                    .unwrap_or(&default_end);
                Ok(serde_json::to_value(
                    self.app_usage.overview(ctx, start, end)?,
                )?)
            }

            "usage_get_hourly" => {
                let default_date = self.app_usage.today_local();
                let date = args
                    .get("date")
                    .and_then(Value::as_str)
                    .unwrap_or(&default_date);
                Ok(serde_json::to_value(self.app_usage.hourly(ctx, date)?)?)
            }

            "usage_get_apps" => Ok(serde_json::to_value(
                self.app_usage.list_tracked_apps(ctx)?,
            )?),

            "usage_get_sessions" => {
                let default_date = self.app_usage.today_local();
                let date = args
                    .get("date")
                    .and_then(Value::as_str)
                    .unwrap_or(&default_date);
                let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50);
                Ok(serde_json::to_value(
                    self.app_usage.sessions(ctx, date, offset, limit)?,
                )?)
            }

            "usage_set_app_category" => {
                let app_id = args
                    .get("app_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CoreError::Config("Missing app_id".to_string()))?;
                let category = args
                    .get("category")
                    .and_then(Value::as_str)
                    .unwrap_or("other");
                let ignored = args
                    .get("ignored")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                Ok(serde_json::to_value(
                    self.app_usage
                        .set_app_category(ctx, app_id, category, ignored)?,
                )?)
            }

            "usage_get_config" => Ok(serde_json::to_value(self.app_usage.config())?),

            "usage_update_config" => {
                let mut config: AppUsageConfig = self.app_usage.config();
                if let Some(idle) = args.get("idle_threshold_secs").and_then(Value::as_u64) {
                    config.idle_threshold_secs = idle;
                }
                if let Some(flush) = args.get("flush_interval_secs").and_then(Value::as_u64) {
                    config.flush_interval_secs = flush;
                }
                if let Some(keep) = args.get("keep_window_titles").and_then(Value::as_bool) {
                    config.keep_window_titles = keep;
                }
                if let Some(audio) = args.get("audio_afk_exception").and_then(Value::as_bool) {
                    config.audio_afk_exception = audio;
                }
                config = config.normalized();
                config
                    .validate()
                    .map_err(|reason| CoreError::Config(reason))?;
                let value = serde_json::to_value(&config)?;
                self.kernel.update_config(|app| {
                    app.modules
                        .settings
                        .insert("app_usage".to_string(), value.clone());
                })?;
                let applied = self.app_usage.reload_config(ctx)?;
                Ok(serde_json::to_value(applied)?)
            }

            _ => Err(CoreError::Config(format!(
                "Unknown app usage command: {cmd}"
            ))),
        }
    }
}
