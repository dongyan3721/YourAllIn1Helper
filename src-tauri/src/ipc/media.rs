//! Feature 08 IPC adapter. Submission returns immediately; workers report progress by events/polling.
use super::*;
use mod_media_proc::{AvJobRequest, ImageConvertOptions};
use std::process::Command;

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
            "media_pick_files" => Ok(serde_json::to_value(native_pick(ctx, false)?)?),
            "media_pick_directory" => {
                let paths = native_pick(ctx, true)?;
                Ok(serde_json::to_value(paths.into_iter().next())?)
            }
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

#[cfg(target_os = "macos")]
fn native_pick(ctx: &app_core::ModuleContext, directory: bool) -> CoreResult<Vec<PathBuf>> {
    let binary = ctx
        .env_probe
        .require_executable(ExternalToolKind::Osascript)?;
    let script = if directory {
        "return POSIX path of (choose folder with prompt \"Select output folder\")"
    } else {
        "set selectedFiles to choose file with prompt \"Select media files\" with multiple selections allowed\nset resultText to \"\"\nrepeat with selectedFile in selectedFiles\nset resultText to resultText & POSIX path of selectedFile & linefeed\nend repeat\nreturn resultText"
    };
    run_dialog(Command::new(binary).args(["-e", script]))
}

#[cfg(target_os = "windows")]
fn native_pick(ctx: &app_core::ModuleContext, directory: bool) -> CoreResult<Vec<PathBuf>> {
    let binary = ctx
        .env_probe
        .require_executable(ExternalToolKind::PowerShell)?;
    let script = if directory {
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.FolderBrowserDialog; if($d.ShowDialog()-eq 'OK'){$d.SelectedPath}"
    } else {
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Multiselect=$true; $d.Filter='Media files|*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.ico;*.mp3;*.wav;*.flac;*.aac;*.m4a;*.ogg;*.mp4;*.mkv;*.mov;*.webm|All files|*.*'; if($d.ShowDialog()-eq 'OK'){$d.FileNames}"
    };
    run_dialog(Command::new(binary).args(["-NoProfile", "-STA", "-Command", script]))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn native_pick(_ctx: &app_core::ModuleContext, _directory: bool) -> CoreResult<Vec<PathBuf>> {
    Err(CoreError::Config(
        "native file selection is not supported on this platform; use drag and drop".into(),
    ))
}

fn run_dialog(command: &mut Command) -> CoreResult<Vec<PathBuf>> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output()?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        if message.contains("User canceled") || message.contains("用户已取消") {
            return Ok(Vec::new());
        }
        return Err(CoreError::Config(format!(
            "native file dialog failed: {}",
            message.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect())
}
