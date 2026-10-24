use crate::{output::finalize_output, types::AvJobRequest};
use app_core::{CoreError, CoreResult, ExternalToolKind, ModuleContext};
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

const MAX_LOG_BYTES: usize = 64 * 1024;
const MAX_PROBE_BYTES: usize = 2 * 1024 * 1024;

pub type ChildSlot = Arc<Mutex<Option<Child>>>;

pub fn probe_file(
    ctx: &ModuleContext,
    input: &Path,
    cancelled: &AtomicBool,
    child_slot: &ChildSlot,
) -> CoreResult<Value> {
    if cancelled.load(Ordering::Acquire) {
        return Err(CoreError::Config("job cancelled".into()));
    }
    let binary = ctx
        .env_probe
        .require_executable(ExternalToolKind::Ffprobe)?;
    let mut child = Command::new(binary)
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    publish_child(child_slot, child, cancelled);
    let stdout_reader = spawn_bounded_reader(stdout, MAX_PROBE_BYTES);
    let stderr_reader = spawn_bounded_reader(stderr, MAX_LOG_BYTES);
    let status = wait_for_child(child_slot)?;
    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(CoreError::Config("job cancelled".into()));
    }
    if !status.success() {
        return Err(CoreError::Config(format!(
            "ffprobe failed: {}",
            concise_error(&stderr)
        )));
    }
    serde_json::from_str(&stdout).map_err(|error| {
        CoreError::Config(format!("ffprobe returned invalid JSON: {error}; {stdout}"))
    })
}

pub fn probe_duration(
    ctx: &ModuleContext,
    input: &Path,
    cancelled: &AtomicBool,
    child_slot: &ChildSlot,
) -> Option<f64> {
    let metadata = probe_file(ctx, input, cancelled, child_slot).ok()?;
    metadata_duration(&metadata)
}

pub fn metadata_duration(metadata: &Value) -> Option<f64> {
    metadata
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            metadata
                .get("streams")?
                .as_array()?
                .iter()
                .filter_map(|stream| stream.get("duration")?.as_str()?.parse::<f64>().ok())
                .max_by(f64::total_cmp)
        })
        .filter(|duration| duration.is_finite() && *duration > 0.0)
}

#[derive(Default)]
pub(crate) struct ProgressLine {
    pub(crate) out_time_us: Option<f64>,
    pub(crate) speed: Option<String>,
    pub(crate) ended: bool,
}

impl ProgressLine {
    pub(crate) fn parse(&mut self, line: &str) {
        let Some((key, value)) = line.split_once('=') else {
            return;
        };
        match key.trim() {
            "out_time_us" | "out_time_ms" => {
                self.out_time_us = value.trim().parse::<f64>().ok().filter(|v| *v >= 0.0)
            }
            "speed" => self.speed = Some(value.trim().to_string()),
            "progress" => self.ended = value.trim() == "end",
            _ => {}
        }
    }
}

pub fn process_av(
    ctx: &ModuleContext,
    request: &AvJobRequest,
    partial: &Path,
    final_output: &Path,
    cancelled: &AtomicBool,
    child_slot: &ChildSlot,
    mut on_progress: impl FnMut(f64, Option<String>) + Send + 'static,
) -> CoreResult<()> {
    request.validate()?;
    let input_duration = probe_duration(ctx, &request.input_path, cancelled, child_slot);
    if cancelled.load(Ordering::Acquire) {
        return Err(CoreError::Config("job cancelled".into()));
    }
    let duration = if request.operation == crate::types::AvOperation::Trim {
        Some(request.end_seconds.unwrap_or(0.0) - request.start_seconds.unwrap_or(0.0))
            .filter(|duration| *duration > 0.0)
    } else {
        input_duration
    };
    let binary = ctx.env_probe.require_executable(ExternalToolKind::Ffmpeg)?;
    let mut command = Command::new(binary);
    command.arg("-nostdin").arg("-hide_banner").arg("-n");
    if request.operation == crate::types::AvOperation::Trim && !request.precise {
        if let Some(start) = request.start_seconds {
            command.arg("-ss").arg(format_seconds(start)?);
        }
    }
    command.arg("-i").arg(&request.input_path);
    match request.operation {
        crate::types::AvOperation::Convert => {}
        crate::types::AvOperation::Trim => {
            if request.precise {
                command
                    .arg("-ss")
                    .arg(format_seconds(request.start_seconds.unwrap_or(0.0))?);
            }
            let start = request.start_seconds.unwrap_or(0.0);
            let end = request.end_seconds.unwrap_or(start);
            command.arg("-t").arg(format_seconds(end - start)?);
            if !request.precise {
                command.args(["-c", "copy"]);
            }
        }
        crate::types::AvOperation::ExtractAudio => {
            command.arg("-vn");
        }
        crate::types::AvOperation::AdjustVolume => {
            command
                .arg("-af")
                .arg(format!("volume={:.6}", request.volume.unwrap_or(1.0)));
        }
    }
    command
        .args(["-loglevel", "error", "-nostats", "-progress", "pipe:1"])
        .arg(partial)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    publish_child(child_slot, child, cancelled);
    let stderr_reader = spawn_bounded_reader(stderr, MAX_LOG_BYTES);
    let progress_reader = thread::spawn(move || -> std::io::Result<()> {
        let Some(stdout) = stdout else {
            return Ok(());
        };
        let mut parsed = ProgressLine::default();
        for line in BufReader::new(stdout).lines() {
            let line = line?;
            parsed.parse(&line);
            if let Some(out_time_us) = parsed.out_time_us {
                let ratio = duration
                    .map(|total| (out_time_us / 1_000_000.0 / total).clamp(0.0, 1.0))
                    .unwrap_or(0.0);
                on_progress(ratio * 99.0, parsed.speed.clone());
                parsed.out_time_us = None;
            }
            if parsed.ended {
                on_progress(99.0, parsed.speed.clone());
            }
        }
        Ok(())
    });
    let status = wait_for_child(child_slot)?;
    progress_reader
        .join()
        .map_err(|_| CoreError::Config("FFmpeg progress reader panicked".into()))??;
    let stderr = join_reader(stderr_reader)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(CoreError::Config("job cancelled".into()));
    }
    if !status.success() {
        return Err(CoreError::Config(format!(
            "ffmpeg failed: {}",
            concise_error(&stderr)
        )));
    }
    finalize_output(partial, final_output)
}

fn publish_child(child_slot: &ChildSlot, child: Child, cancelled: &AtomicBool) {
    *child_slot.lock() = Some(child);
    // Close the spawn/publish race: cancellation may have happened just before publication.
    // The normal wait path still reaps the process and joins both output readers.
    if cancelled.load(Ordering::Acquire) {
        kill_child(child_slot);
    }
}

fn wait_for_child(child_slot: &ChildSlot) -> CoreResult<std::process::ExitStatus> {
    loop {
        {
            let mut guard = child_slot.lock();
            let child = guard
                .as_mut()
                .ok_or_else(|| CoreError::Config("media process handle is missing".into()))?;
            if let Some(status) = child.try_wait()? {
                *guard = None;
                return Ok(status);
            }
        }
        // Never hold the child mutex while waiting: cancellation must be able to kill it.
        thread::sleep(std::time::Duration::from_millis(40));
    }
}

fn spawn_bounded_reader(
    reader: Option<impl Read + Send + 'static>,
    limit: usize,
) -> thread::JoinHandle<std::io::Result<String>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut reader) = reader {
            let mut buffer = [0_u8; 8192];
            loop {
                let count = reader.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() > limit {
                    let overflow = bytes.len() - limit;
                    bytes.drain(..overflow);
                }
            }
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })
}

fn join_reader(handle: thread::JoinHandle<std::io::Result<String>>) -> CoreResult<String> {
    handle
        .join()
        .map_err(|_| CoreError::Config("media process log reader panicked".into()))?
        .map_err(CoreError::from)
}

fn concise_error(log: &str) -> String {
    let trimmed = log.trim();
    if trimmed.is_empty() {
        "unknown process error".into()
    } else {
        trimmed
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn format_seconds(value: f64) -> CoreResult<String> {
    if !value.is_finite() || value < 0.0 {
        return Err(CoreError::Config("invalid time value".into()));
    }
    Ok(format!("{value:.6}"))
}

pub fn kill_child(child_slot: &ChildSlot) {
    if let Some(child) = child_slot.lock().as_mut() {
        let _ = child.kill();
    }
}
