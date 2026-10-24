//! 自定义脚本 IPC 适配：CRUD、解释器探测、执行、取消、日志与执行记录。
use super::*;
use app_core::CoreError;
use mod_script_runner::config::{ScriptRunnerConfig, MODULE_ID};
use mod_script_runner::model::ScriptUpsert;
use serde_json::Value;

fn string_args(args: &Value) -> Vec<String> {
    args.get("args")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl DesktopHost {
    pub(super) fn dispatch_script(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        match cmd {
            "script_probe_interpreters" => {
                let interpreters = self.script_runner.probe_interpreters(ctx);
                Ok(json!({
                    "interpreters": interpreters,
                    "environment": self.script_runner.environment_capabilities(),
                }))
            }

            "script_list" => Ok(serde_json::to_value(self.script_runner.list_scripts(ctx)?)?),

            "script_get" => {
                let id = args
                    .get("script_id")
                    .or_else(|| args.get("id"))
                    .and_then(Value::as_i64)
                    .ok_or_else(|| CoreError::Config("Missing script_id".to_string()))?;
                Ok(serde_json::to_value(
                    self.script_runner.get_script(ctx, id)?,
                )?)
            }

            "script_save" => {
                let payload = args.get("script").cloned().unwrap_or(args);
                let input: ScriptUpsert = serde_json::from_value(payload)?;
                let saved = self.script_runner.save_script(ctx, input)?;
                Ok(serde_json::to_value(saved)?)
            }

            "script_delete" => {
                let id = args
                    .get("script_id")
                    .or_else(|| args.get("id"))
                    .and_then(Value::as_i64)
                    .ok_or_else(|| CoreError::Config("Missing script_id".to_string()))?;
                let deleted = self.script_runner.delete_script(ctx, id)?;
                Ok(json!({ "script_id": id, "deleted": deleted }))
            }

            "script_execute" => {
                let script_id = args
                    .get("script_id")
                    .or_else(|| args.get("id"))
                    .and_then(Value::as_i64)
                    .ok_or_else(|| CoreError::Config("Missing script_id".to_string()))?;
                let extra = string_args(&args);
                let run = self.script_runner.execute(ctx, script_id, extra)?;
                Ok(serde_json::to_value(run)?)
            }

            "script_kill" => {
                let run_id = args
                    .get("run_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CoreError::Config("Missing run_id".to_string()))?;
                let killed = self.script_runner.kill(run_id);
                Ok(json!({
                    "run_id": run_id,
                    "killed": killed,
                    "active_count": self.script_runner.active_run_count(),
                }))
            }

            "script_list_runs" => {
                let script_id = args.get("script_id").and_then(Value::as_i64);
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
                Ok(serde_json::to_value(
                    self.script_runner.list_runs(ctx, script_id, limit)?,
                )?)
            }

            "script_get_run" => {
                let run_id = args
                    .get("run_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CoreError::Config("Missing run_id".to_string()))?;
                Ok(serde_json::to_value(
                    self.script_runner.get_run(ctx, run_id)?,
                )?)
            }

            "script_get_logs" => {
                let run_id = args
                    .get("run_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CoreError::Config("Missing run_id".to_string()))?;
                let since_seq = args.get("since_seq").and_then(Value::as_u64).unwrap_or(0);
                match self.script_runner.get_logs(ctx, run_id, since_seq)? {
                    Some((entries, run)) => Ok(json!({
                        "entries": entries,
                        "run": run,
                        "live": run.status == mod_script_runner::model::RunStatus::Running,
                    })),
                    None => Ok(json!({
                        "entries": [],
                        "run": null,
                        "live": false,
                    })),
                }
            }

            "script_get_settings" => Ok(serde_json::to_value(self.script_runner.settings(ctx)?)?),

            "script_update_settings" => {
                let payload = args.get("settings").cloned().unwrap_or(args);
                let config: ScriptRunnerConfig = serde_json::from_value(payload)?;
                config.validate().map_err(CoreError::Config)?;
                let config = config.normalized();
                self.kernel.update_config(|cfg| {
                    cfg.modules.settings.insert(
                        MODULE_ID.to_string(),
                        serde_json::to_value(&config).expect("config serializes"),
                    );
                })?;
                Ok(serde_json::to_value(config)?)
            }

            _ => Err(CoreError::Config(format!("Unknown IPC command: {}", cmd))),
        }
    }
}
