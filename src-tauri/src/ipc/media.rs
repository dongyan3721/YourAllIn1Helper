//! Feature 08 IPC adapter. Submission returns immediately; workers report progress by events/polling.
use super::*;
use mod_media_proc::{AvJobRequest, ImageConvertOptions};

#[derive(Deserialize)]
struct ImageSubmitRequest {
    input_paths: Vec<PathBuf>,
    output_dir: Option<PathBuf>,
    options: ImageConvertOptions,
}

impl DesktopHost {
    pub(super) fn dispatch_media(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        match cmd {
            "media_check_ffmpeg_env" => {
                let refresh = args.get("refresh").and_then(Value::as_bool).unwrap_or(true);
                Ok(self.media_proc.check_environment(ctx, refresh))
            }
            "media_get_config" => Ok(serde_json::to_value(self.media_proc.settings(ctx)?)?),
            "media_update_config" => {
                let mut settings = self.media_proc.settings(ctx)?;
                if args.get("output_dir").is_some_and(Value::is_null) {
                    settings.output_dir = None;
                } else if let Some(path) = args.get("output_dir").and_then(Value::as_str) {
                    let trimmed = path.trim();
                    settings.output_dir = (!trimmed.is_empty()).then(|| PathBuf::from(trimmed));
                }
                if let Some(limit) = args.get("max_concurrent_jobs").and_then(Value::as_u64) {
                    settings.max_concurrent_jobs = usize::try_from(limit).map_err(|_| {
                        CoreError::Config("max_concurrent_jobs is too large".into())
                    })?;
                }
                settings.validate()?;
                let value = serde_json::to_value(&settings)?;
                self.kernel.update_config(|config| {
                    config
                        .modules
                        .settings
                        .insert("media_proc".into(), value.clone());
                })?;
                self.media_proc.apply_settings(&settings)?;
                Ok(serde_json::to_value(settings)?)
            }
            "media_convert_images" => {
                let request: ImageSubmitRequest = serde_json::from_value(args)?;
                let jobs = self.media_proc.submit_image_jobs(
                    ctx,
                    request.input_paths,
                    request.output_dir,
                    request.options,
                )?;
                Ok(serde_json::to_value(jobs)?)
            }
            "media_submit_av_job" => {
                let request: AvJobRequest = serde_json::from_value(args)?;
                Ok(serde_json::to_value(
                    self.media_proc.submit_av_job(ctx, request)?,
                )?)
            }
            "media_inspect_file" => {
                let input = required_path(&args, "input_path")?;
                Ok(serde_json::to_value(
                    self.media_proc.inspect_file(ctx, input)?,
                )?)
            }
            "media_list_jobs" => Ok(serde_json::to_value(self.media_proc.list_jobs(ctx)?)?),
            "media_cancel_job" => {
                let id = args
                    .get("job_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CoreError::Config("Missing job_id".into()))?;
                Ok(serde_json::to_value(self.media_proc.cancel_job(ctx, id)?)?)
            }
            "media_pick_files" | "media_pick_directory" => Err(CoreError::Config(
                "native media selection requires the asynchronous desktop IPC adapter".into(),
            )),
            _ => Err(CoreError::Config(format!("Unknown IPC command: {}", cmd))),
        }
    }
}

fn required_path(args: &Value, key: &str) -> CoreResult<PathBuf> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| CoreError::Config(format!("Missing {key}")))
}
