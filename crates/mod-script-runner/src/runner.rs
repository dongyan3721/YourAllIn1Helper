//! 子进程执行引擎：argv 直接执行、增量日志、超时/取消杀进程、日志缓存上限。
use crate::{
    command::{apply_platform_options, build_command, inline_file_name},
    env::ScriptEnvironment,
    events::ScriptEventSink,
    model::{LogEntry, LogStream, RunStatus, ScriptRecord, ScriptRunRecord, ScriptType},
    store,
};
use app_core::{CoreError, CoreResult, ModuleContext};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tracing::{error, info};

const TAIL_BYTES: usize = 32 * 1024;
const READ_CHUNK: usize = 4096;
const ENV_VALUE_BYTES: usize = 64 * 1024;

pub(crate) struct ActiveRun {
    child: Arc<Mutex<Child>>,
    cancel: Arc<AtomicBool>,
    state: Arc<RunState>,
}

pub(crate) struct RunState {
    run_id: String,
    script_id: i64,
    script_name: String,
    pid: Option<u32>,
    started_at: i64,
    started_instant: Instant,
    live_enabled: bool,
    status: Mutex<RunStatus>,
    exit_code: Mutex<Option<i32>>,
    log: Mutex<LogBuffer>,
}

struct LogBuffer {
    entries: Vec<LogEntry>,
    bytes: usize,
    dropped_bytes: usize,
    next_seq: u64,
    max_bytes: usize,
}

impl LogBuffer {
    fn new(max_bytes: usize) -> Self {
        Self {
            entries: Vec::new(),
            bytes: 0,
            dropped_bytes: 0,
            next_seq: 1,
            max_bytes,
        }
    }

    fn append(&mut self, stream: LogStream, mut text: String, at: i64) -> Option<LogEntry> {
        if text.is_empty() {
            return None;
        }
        if text.len() > self.max_bytes {
            let dropped = text.len() - self.max_bytes;
            truncate_front_utf8(&mut text, self.max_bytes);
            self.dropped_bytes += dropped;
        }
        let entry = LogEntry {
            seq: self.next_seq,
            stream,
            text,
            at,
        };
        self.next_seq += 1;
        self.bytes += entry.text.len();
        self.entries.push(entry.clone());
        while self.bytes > self.max_bytes && self.entries.len() > 1 {
            let removed = self.entries.remove(0);
            self.bytes -= removed.text.len();
            self.dropped_bytes += removed.text.len();
        }
        if self.bytes > self.max_bytes {
            if let Some(first) = self.entries.first_mut() {
                let excess = self.bytes - self.max_bytes;
                let keep = first.text.len().saturating_sub(excess);
                truncate_front_utf8(&mut first.text, keep);
                self.bytes = self.entries.iter().map(|e| e.text.len()).sum();
                self.dropped_bytes += excess;
            }
        }
        Some(entry)
    }

    fn entries_since(&self, seq: u64) -> Vec<LogEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.seq > seq)
            .cloned()
            .collect()
    }

    fn tail(&self, stream: LogStream) -> String {
        let mut combined = String::new();
        for entry in self.entries.iter().filter(|entry| entry.stream == stream) {
            combined.push_str(&entry.text);
        }
        if combined.len() > TAIL_BYTES {
            truncate_front_utf8(&mut combined, TAIL_BYTES);
        }
        combined
    }

    fn bytes(&self) -> usize {
        self.bytes
    }

    fn dropped_bytes(&self) -> usize {
        self.dropped_bytes
    }
}

impl RunState {
    fn live_record(&self) -> ScriptRunRecord {
        let log = self.log.lock();
        let status = *self.status.lock();
        let exit_code = *self.exit_code.lock();
        ScriptRunRecord {
            run_id: self.run_id.clone(),
            script_id: self.script_id,
            script_name: self.script_name.clone(),
            status,
            exit_code,
            timed_out: status == RunStatus::TimedOut,
            pid: self.pid,
            started_at: self.started_at,
            finished_at: None,
            duration_ms: Some(self.started_instant.elapsed().as_millis() as u64),
            stdout_tail: log.tail(LogStream::Stdout),
            stderr_tail: log.tail(LogStream::Stderr),
            error_message: None,
            log_bytes: log.bytes(),
            dropped_bytes: log.dropped_bytes(),
        }
    }
}

/// 模块级执行状态，可被 IPC 与后台监控线程共享。
pub(crate) struct Engine {
    environment: Arc<dyn ScriptEnvironment>,
    events: Arc<dyn ScriptEventSink>,
    active: Mutex<HashMap<String, ActiveRun>>,
    shutting_down: AtomicBool,
}

impl Engine {
    pub(crate) fn new(
        environment: Arc<dyn ScriptEnvironment>,
        events: Arc<dyn ScriptEventSink>,
    ) -> Self {
        Self {
            environment,
            events,
            active: Mutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
        }
    }

    pub(crate) fn environment(&self) -> &Arc<dyn ScriptEnvironment> {
        &self.environment
    }

    pub(crate) fn start_running(&self) {
        self.shutting_down.store(false, Ordering::SeqCst);
    }

    pub(crate) fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    pub(crate) fn execute(
        self: &Arc<Self>,
        ctx: &ModuleContext,
        script: &ScriptRecord,
        extra_args: &[String],
        timeout_secs: u64,
        max_log_bytes: usize,
        keep_run_logs: usize,
    ) -> CoreResult<ScriptRunRecord> {
        if self.is_shutting_down() {
            return Err(CoreError::ModuleDisabled(
                crate::config::MODULE_ID.to_string(),
            ));
        }

        let interpreter = crate::probe_for(ctx, script.script_type)?;
        if !interpreter.supported {
            return Err(CoreError::Config(format!(
                "{} scripts are not supported on this operating system",
                script.script_type.as_str()
            )));
        }
        let interpreter_path = interpreter.resolved_path.clone().ok_or_else(|| {
            CoreError::ExternalToolUnavailable {
                tool: script.script_type.as_str().to_string(),
                hint: interpreter.install_hint.clone(),
            }
        })?;

        let run_id = uuid::Uuid::new_v4().to_string();
        let runtime_dir = ctx.app_data_dir.join("script_runtime").join(&run_id);
        let (script_path, cleanup_dir) = self.materialize_script(script, &runtime_dir)?;

        let working_dir = match script.working_dir.as_deref().map(str::trim) {
            Some(dir) if !dir.is_empty() => {
                let dir = expand_path(dir);
                if !dir.is_dir() {
                    cleanup(&cleanup_dir);
                    return Err(CoreError::Config(format!(
                        "working directory does not exist: {}",
                        dir.display()
                    )));
                }
                dir
            }
            _ => {
                if script.is_inline {
                    ctx.app_data_dir.clone()
                } else {
                    Path::new(&script_path)
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| ctx.app_data_dir.clone())
                }
            }
        };

        let mut args = script.args.clone();
        args.extend(extra_args.iter().cloned());

        let built = build_command(
            script.script_type,
            PathBuf::from(&interpreter_path),
            &script_path,
            &args,
        );

        let mut command = Command::new(&built.program);
        command
            .args(&built.args)
            .current_dir(&working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PYTHONUNBUFFERED", "1");

        if self.environment.capabilities().clipboard {
            if let Ok(Some(value)) = self.environment.clipboard_text() {
                if !value.is_empty() {
                    command.env("AIO_CLIPBOARD", truncate_env_value(value));
                }
            }
        }
        if self.environment.capabilities().selection {
            if let Ok(Some(value)) = self.environment.selected_text() {
                if !value.is_empty() {
                    command.env("AIO_SELECTION", truncate_env_value(value));
                }
            }
        }
        apply_platform_options(&mut command);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                cleanup(&cleanup_dir);
                let message = format!("failed to spawn {}: {err}", built.program.display());
                let _ = store::mark_spawn_failed(ctx, &run_id, script, &message);
                return Err(CoreError::Io(std::io::Error::new(err.kind(), message)));
            }
        };
        let pid = child.id();
        let started_at = chrono::Utc::now().timestamp_millis();
        let emit_stream = script.output_handling != "final" && script.output_handling != "discard";

        if let Err(err) = store::insert_running_run(ctx, &run_id, script, Some(pid)) {
            let _ = child.kill();
            cleanup(&cleanup_dir);
            return Err(err);
        }

        let state = Arc::new(RunState {
            run_id: run_id.clone(),
            script_id: script.id,
            script_name: script.name.clone(),
            pid: Some(pid),
            started_at,
            started_instant: Instant::now(),
            live_enabled: emit_stream,
            status: Mutex::new(RunStatus::Running),
            exit_code: Mutex::new(None),
            log: Mutex::new(LogBuffer::new(max_log_bytes)),
        });

        let child = Arc::new(Mutex::new(child));
        let cancel = Arc::new(AtomicBool::new(false));

        let stdout = child.lock().stdout.take();
        let stderr = child.lock().stderr.take();
        let mut readers = Vec::new();
        if let Some(stdout) = stdout {
            readers.push(spawn_reader(
                stdout,
                LogStream::Stdout,
                Arc::clone(&state),
                Arc::clone(&self.events),
                emit_stream,
            ));
        }
        if let Some(stderr) = stderr {
            readers.push(spawn_reader(
                stderr,
                LogStream::Stderr,
                Arc::clone(&state),
                Arc::clone(&self.events),
                emit_stream,
            ));
        }

        self.active.lock().insert(
            run_id.clone(),
            ActiveRun {
                child: Arc::clone(&child),
                cancel: Arc::clone(&cancel),
                state: Arc::clone(&state),
            },
        );

        let running_record = state.live_record();
        let engine = Arc::clone(self);
        let ctx = ctx.clone();
        thread::spawn(move || {
            monitor_run(
                engine,
                ctx,
                state,
                child,
                cancel,
                readers,
                timeout_secs.max(1),
                keep_run_logs,
                cleanup_dir,
            );
        });

        Ok(running_record)
    }

    fn materialize_script(
        &self,
        script: &ScriptRecord,
        runtime_dir: &Path,
    ) -> CoreResult<(String, Option<PathBuf>)> {
        if script.is_inline {
            if script.content_or_path.trim().is_empty() {
                return Err(CoreError::Config("script content is empty".to_string()));
            }
            std::fs::create_dir_all(runtime_dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(runtime_dir, std::fs::Permissions::from_mode(0o700));
            }
            let path = runtime_dir.join(inline_file_name(script.script_type));
            let mut content = script.content_or_path.clone();
            if script.script_type == ScriptType::Batch {
                content = content.replace("\r\n", "\n").replace('\n', "\r\n");
            }
            std::fs::write(&path, content)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            Ok((
                path.to_string_lossy().into_owned(),
                Some(runtime_dir.to_path_buf()),
            ))
        } else {
            let path = expand_path(script.content_or_path.trim());
            if !path.is_file() {
                return Err(CoreError::Config(format!(
                    "script file does not exist: {}",
                    path.display()
                )));
            }
            Ok((path.to_string_lossy().into_owned(), None))
        }
    }

    pub(crate) fn kill(&self, run_id: &str) -> bool {
        let active = self.active.lock();
        let Some(run) = active.get(run_id) else {
            return false;
        };
        run.cancel.store(true, Ordering::SeqCst);
        terminate(&mut run.child.lock());
        true
    }

    pub(crate) fn get_run(
        &self,
        ctx: &ModuleContext,
        run_id: &str,
    ) -> CoreResult<Option<ScriptRunRecord>> {
        if let Some(run) = self.active.lock().get(run_id) {
            return Ok(Some(run.state.live_record()));
        }
        store::get_run(ctx, run_id)
    }

    pub(crate) fn get_logs(
        &self,
        ctx: &ModuleContext,
        run_id: &str,
        since_seq: u64,
    ) -> CoreResult<Option<(Vec<LogEntry>, ScriptRunRecord)>> {
        if let Some(run) = self.active.lock().get(run_id) {
            let entries = if run.state.live_enabled {
                run.state.log.lock().entries_since(since_seq)
            } else {
                Vec::new()
            };
            return Ok(Some((entries, run.state.live_record())));
        }
        store::get_run(ctx, run_id).map(|record| record.map(|record| (Vec::new(), record)))
    }

    pub(crate) fn active_count(&self) -> usize {
        self.active.lock().len()
    }

    pub(crate) fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let runs: Vec<Arc<Mutex<Child>>> = {
            let active = self.active.lock();
            active
                .values()
                .map(|run| {
                    run.cancel.store(true, Ordering::SeqCst);
                    Arc::clone(&run.child)
                })
                .collect()
        };
        for child in &runs {
            terminate(&mut child.lock());
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.active_count() > 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
    }
}

fn monitor_run(
    engine: Arc<Engine>,
    ctx: ModuleContext,
    state: Arc<RunState>,
    child: Arc<Mutex<Child>>,
    cancel: Arc<AtomicBool>,
    readers: Vec<JoinHandle<()>>,
    timeout_secs: u64,
    keep_run_logs: usize,
    cleanup_dir: Option<PathBuf>,
) {
    let timeout = Duration::from_secs(timeout_secs);
    let mut timed_out = false;
    let mut cancelled = false;
    let mut error_message: Option<String> = None;
    let exit_code = loop {
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
            terminate(&mut child.lock());
        } else if state.started_instant.elapsed() >= timeout {
            timed_out = true;
            terminate(&mut child.lock());
        }
        let waited = child.lock().try_wait();
        match waited {
            Ok(Some(status)) => break status.code(),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(err) => {
                error_message = Some(format!("failed to wait for script process: {err}"));
                terminate(&mut child.lock());
                break None;
            }
        }
    };

    for reader in readers {
        let _ = reader.join();
    }

    let status = if timed_out {
        RunStatus::TimedOut
    } else if cancelled {
        RunStatus::Cancelled
    } else if exit_code == Some(0) {
        RunStatus::Succeeded
    } else {
        RunStatus::Failed
    };
    *state.status.lock() = status;
    *state.exit_code.lock() = exit_code;

    let finished_at = chrono::Utc::now().timestamp_millis();
    let duration_ms = state.started_instant.elapsed().as_millis() as u64;
    let record = {
        let log = state.log.lock();
        ScriptRunRecord {
            run_id: state.run_id.clone(),
            script_id: state.script_id,
            script_name: state.script_name.clone(),
            status,
            exit_code,
            timed_out,
            pid: state.pid,
            started_at: state.started_at,
            finished_at: Some(finished_at),
            duration_ms: Some(duration_ms),
            stdout_tail: log.tail(LogStream::Stdout),
            stderr_tail: log.tail(LogStream::Stderr),
            error_message,
            log_bytes: log.bytes(),
            dropped_bytes: log.dropped_bytes(),
        }
    };

    if let Err(err) = store::finish_run(&ctx, &record, keep_run_logs) {
        error!(run_id = %record.run_id, %err, "Failed to persist script execution record");
    }
    if let Some(dir) = cleanup_dir {
        cleanup(&Some(dir));
    }
    engine.active.lock().remove(&record.run_id);
    engine.events.on_finished(&record.summary());
    info!(
        run_id = %record.run_id,
        status = ?record.status,
        exit_code = ?record.exit_code,
        "Script execution finished"
    );
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    stream: LogStream,
    state: Arc<RunState>,
    events: Arc<dyn ScriptEventSink>,
    emit_stream: bool,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0u8; READ_CHUNK];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buffer[..n]).into_owned();
                    let at = chrono::Utc::now().timestamp_millis();
                    let entry = {
                        let mut log = state.log.lock();
                        log.append(stream, text, at)
                    };
                    if emit_stream {
                        if let Some(entry) = entry {
                            events.on_log(&state.run_id, &entry);
                        }
                    }
                }
                Err(_) => break,
            }
        }
    })
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
}

fn cleanup(dir: &Option<PathBuf>) {
    if let Some(dir) = dir {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn expand_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn truncate_env_value(mut value: String) -> String {
    if value.len() > ENV_VALUE_BYTES {
        truncate_front_utf8(&mut value, ENV_VALUE_BYTES);
    }
    value
}

fn truncate_front_utf8(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut cut = value.len() - max_bytes;
    while cut < value.len() && !value.is_char_boundary(cut) {
        cut += 1;
    }
    value.drain(..cut);
}
