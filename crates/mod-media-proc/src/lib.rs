//! Feature 08: real Rust image codecs and system FFmpeg/FFprobe job processing.
//! No runtime is installed or bundled; every long-running operation executes on a module-owned worker.

mod av;
pub mod config;
mod image;
mod output;
mod semaphore;
mod types;

pub use config::MediaProcConfig;
pub use types::{
    AvJobRequest, AvOperation, ImageConvertOptions, ImageOutputFormat, MediaJob, MediaJobStatus,
};

use app_core::{
    AppModule, CoreError, CoreResult, ExternalToolKind, ModuleContext, ModuleMigration,
};
use parking_lot::Mutex;
use rusqlite::params;
use semaphore::JobSemaphore;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Child,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};

pub const MODULE_ID: &str = "media_proc";
const MAX_BATCH_FILES: usize = 100;

type EventSink = Arc<dyn Fn(MediaJob) + Send + Sync + 'static>;

#[derive(Clone)]
struct ActiveJob {
    job: MediaJob,
    cancelled: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    partial: Option<PathBuf>,
}

#[derive(Clone)]
enum JobWork {
    Image(ImageConvertOptions),
    Av(AvJobRequest),
    Inspect,
}

pub struct MediaProcModule {
    runtime: Mutex<Option<ModuleContext>>,
    active: Mutex<HashMap<String, ActiveJob>>,
    workers: Mutex<HashMap<String, JoinHandle<()>>>,
    accepting: AtomicBool,
    semaphore: JobSemaphore,
    event_sink: Mutex<Option<EventSink>>,
}

impl Default for MediaProcModule {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaProcModule {
    pub fn new() -> Self {
        Self {
            runtime: Mutex::new(None),
            active: Mutex::new(HashMap::new()),
            workers: Mutex::new(HashMap::new()),
            accepting: AtomicBool::new(false),
            semaphore: JobSemaphore::new(2),
            event_sink: Mutex::new(None),
        }
    }

    pub fn set_event_sink(&self, sink: Option<EventSink>) {
        *self.event_sink.lock() = sink;
    }

    pub fn check_environment(&self, ctx: &ModuleContext, refresh: bool) -> Value {
        let ffmpeg = ctx.env_probe.probe(ExternalToolKind::Ffmpeg, refresh);
        let ffprobe = ctx.env_probe.probe(ExternalToolKind::Ffprobe, refresh);
        json!({
            "ffmpeg": ffmpeg,
            "ffprobe": ffprobe,
            "image_formats": ["png", "jpeg", "webp", "bmp", "ico"],
            "av_operations": ["inspect", "convert", "trim", "extract_audio", "adjust_volume"]
        })
    }

    pub fn settings(&self, ctx: &ModuleContext) -> CoreResult<MediaProcConfig> {
        let settings: MediaProcConfig = ctx.module_settings(MODULE_ID)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn apply_settings(&self, settings: &MediaProcConfig) -> CoreResult<()> {
        settings.validate()?;
        self.semaphore.set_limit(settings.max_concurrent_jobs);
        Ok(())
    }

    pub fn submit_image_jobs(
        self: &Arc<Self>,
        ctx: &ModuleContext,
        input_paths: Vec<PathBuf>,
        output_dir: Option<PathBuf>,
        options: ImageConvertOptions,
    ) -> CoreResult<Vec<MediaJob>> {
        self.ensure_accepting()?;
        options.validate()?;
        validate_batch(&input_paths)?;
        let configured_dir = self.resolve_output_dir(ctx, output_dir)?;
        let mut prepared = Vec::with_capacity(input_paths.len());
        for input in input_paths {
            let mut job = MediaJob::new("image_convert", input);
            match output::prepare_output(
                &job.input_path,
                configured_dir.as_deref(),
                options.output_format.extension(),
                &job.id,
            ) {
                Ok((output, partial)) => {
                    job.output_path = Some(output);
                    prepared.push((job, partial));
                }
                Err(error) => {
                    for (job, _) in prepared {
                        if let Some(output) = &job.output_path {
                            output::release_output(output);
                        }
                    }
                    return Err(error);
                }
            }
        }
        let mut submitted: Vec<MediaJob> = Vec::with_capacity(prepared.len());
        let mut prepared = prepared.into_iter();
        while let Some((job, partial)) = prepared.next() {
            if let Err(error) = self.spawn_job(
                ctx,
                job.clone(),
                Some(partial),
                JobWork::Image(options.clone()),
            ) {
                for previous in &submitted {
                    let _ = self.cancel_job(ctx, previous.id.as_str());
                }
                for (remaining, _) in prepared {
                    if let Some(output) = &remaining.output_path {
                        output::release_output(output);
                    }
                }
                return Err(error);
            }
            submitted.push(job);
        }
        Ok(submitted)
    }

    pub fn submit_av_job(
        self: &Arc<Self>,
        ctx: &ModuleContext,
        request: AvJobRequest,
    ) -> CoreResult<MediaJob> {
        self.ensure_accepting()?;
        request.validate()?;
        ctx.env_probe.require_executable(ExternalToolKind::Ffmpeg)?;
        ctx.env_probe
            .require_executable(ExternalToolKind::Ffprobe)?;
        let configured_dir = self.resolve_output_dir(ctx, request.output_dir.clone())?;
        let extension = request
            .output_format
            .as_deref()
            .map(|format| format.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|format| !format.is_empty())
            .or_else(|| {
                request
                    .input_path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase)
            })
            .ok_or_else(|| CoreError::Config("cannot determine output extension".into()))?;
        let mut job = MediaJob::new(request.operation.as_str(), request.input_path.clone());
        let (output, partial) = output::prepare_output(
            &request.input_path,
            configured_dir.as_deref(),
            &extension,
            &job.id,
        )?;
        job.output_path = Some(output);
        self.spawn_job(ctx, job.clone(), Some(partial), JobWork::Av(request))?;
        Ok(job)
    }

    pub fn inspect_file(
        self: &Arc<Self>,
        ctx: &ModuleContext,
        input_path: PathBuf,
    ) -> CoreResult<MediaJob> {
        self.ensure_accepting()?;
        if !input_path.is_file() {
            return Err(CoreError::Config(format!(
                "input file does not exist: {}",
                input_path.display()
            )));
        }
        let job = MediaJob::new("inspect", input_path);
        self.spawn_job(ctx, job.clone(), None, JobWork::Inspect)?;
        Ok(job)
    }

    pub fn cancel_job(&self, ctx: &ModuleContext, job_id: &str) -> CoreResult<Option<MediaJob>> {
        let active = self.active.lock().get(job_id).cloned();
        if let Some(active) = active {
            active.cancelled.store(true, Ordering::Release);
            av::kill_child(&active.child);
            self.semaphore.wake_all();
            return Ok(Some(active.job));
        }
        Ok(self.job_snapshot(ctx, job_id).ok())
    }

    pub fn list_jobs(&self, ctx: &ModuleContext) -> CoreResult<Vec<MediaJob>> {
        self.reap_finished_workers();
        let mut jobs = ctx.db.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT id, operation, input_path, output_path, status, progress, speed,
                        error, metadata_json, created_at, updated_at
                 FROM media_task_history ORDER BY created_at DESC LIMIT 1000",
            )?;
            let rows = statement.query_map([], map_job_row)?;
            let mut jobs = Vec::new();
            for row in rows {
                jobs.push(row?);
            }
            Ok(jobs)
        })?;
        let active = self.active.lock();
        for job in &mut jobs {
            if let Some(current) = active.get(&job.id) {
                *job = current.job.clone();
            }
        }
        jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(jobs)
    }

    pub fn active_job_count(&self) -> usize {
        self.active.lock().len()
    }

    pub fn worker_count(&self) -> usize {
        self.workers.lock().len()
    }

    fn reap_finished_workers(&self) {
        let finished: Vec<String> = self
            .workers
            .lock()
            .iter()
            .filter(|(_, worker)| worker.is_finished())
            .map(|(id, _)| id.clone())
            .collect();
        for id in finished {
            if let Some(worker) = self.workers.lock().remove(&id) {
                let _ = worker.join();
            }
        }
    }

    fn ensure_accepting(&self) -> CoreResult<()> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(CoreError::ModuleDisabled(MODULE_ID.into()));
        }
        Ok(())
    }

    fn resolve_output_dir(
        &self,
        ctx: &ModuleContext,
        requested: Option<PathBuf>,
    ) -> CoreResult<Option<PathBuf>> {
        if requested.is_some() {
            return Ok(requested);
        }
        Ok(self.settings(ctx)?.output_dir)
    }

    fn spawn_job(
        self: &Arc<Self>,
        ctx: &ModuleContext,
        job: MediaJob,
        partial: Option<PathBuf>,
        work: JobWork,
    ) -> CoreResult<()> {
        if let Err(error) = self.ensure_accepting() {
            if let Some(partial) = &partial {
                output::remove_partial(partial);
            }
            if let Some(output) = &job.output_path {
                output::release_output(output);
            }
            return Err(error);
        }
        if let Err(error) = persist_job(ctx, &job) {
            if let Some(partial) = &partial {
                output::remove_partial(partial);
            }
            if let Some(output) = &job.output_path {
                output::release_output(output);
            }
            return Err(error);
        }
        let active = ActiveJob {
            job: job.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            partial,
        };
        self.active.lock().insert(job.id.clone(), active.clone());
        let module = Arc::clone(self);
        let worker_ctx = ctx.clone();
        let worker_job = job.clone();
        let worker_active = active.clone();
        match thread::Builder::new()
            .name(format!("media-{}", job.id))
            .spawn(move || module.run_job(worker_ctx, worker_job, worker_active, work))
        {
            Ok(handle) => {
                self.workers.lock().insert(job.id, handle);
                Ok(())
            }
            Err(error) => {
                self.active.lock().remove(&job.id);
                if let Some(partial) = &active.partial {
                    output::remove_partial(partial);
                }
                if let Some(output) = &job.output_path {
                    output::release_output(output);
                }
                let mut failed = job;
                failed.status = MediaJobStatus::Failed;
                failed.error = Some(format!("could not start media worker: {error}"));
                failed.updated_at = chrono::Utc::now().timestamp_millis();
                let _ = persist_job(ctx, &failed);
                Err(CoreError::from(error))
            }
        }
    }

    fn run_job(
        self: Arc<Self>,
        ctx: ModuleContext,
        job: MediaJob,
        active: ActiveJob,
        work: JobWork,
    ) {
        struct ReleaseOutput(Option<PathBuf>);
        impl Drop for ReleaseOutput {
            fn drop(&mut self) {
                if let Some(output) = &self.0 {
                    output::release_output(output);
                }
            }
        }
        let _output_reservation = ReleaseOutput(job.output_path.clone());
        if !self.semaphore.acquire(&active.cancelled) {
            self.finish_job(
                &ctx,
                &job.id,
                MediaJobStatus::Cancelled,
                None,
                Some("job cancelled".into()),
                None,
            );
            self.active.lock().remove(&job.id);
            return;
        }
        struct Permit<'a>(&'a JobSemaphore);
        impl Drop for Permit<'_> {
            fn drop(&mut self) {
                self.0.release();
            }
        }
        let _permit = Permit(&self.semaphore);
        self.update_active(&job.id, |current| {
            current.status = MediaJobStatus::Running;
            current.progress = 0.0;
            current.updated_at = chrono::Utc::now().timestamp_millis();
        });
        if let Some(current) = self.active.lock().get(&job.id).cloned() {
            let _ = persist_job(&ctx, &current.job);
        }

        let result = match work {
            JobWork::Image(options) => image::process(
                &job,
                &options,
                active.partial.as_deref().unwrap_or_else(|| Path::new("")),
                &active.cancelled,
            ),
            JobWork::Av(request) => {
                let final_output = job.output_path.clone().unwrap_or_default();
                let module = self.clone();
                let id = job.id.clone();
                av::process_av(
                    &ctx,
                    &request,
                    active.partial.as_deref().unwrap_or_else(|| Path::new("")),
                    &final_output,
                    &active.cancelled,
                    &active.child,
                    move |progress, speed| {
                        module.update_active(&id, |current| {
                            current.progress = progress;
                            current.speed = speed;
                            current.updated_at = chrono::Utc::now().timestamp_millis();
                        });
                    },
                )
            }
            JobWork::Inspect => match ::image::image_dimensions(&job.input_path) {
                Ok((width, height)) => {
                    let format = ::image::ImageReader::open(&job.input_path)
                        .and_then(|reader| reader.with_guessed_format())
                        .ok()
                        .and_then(|reader| reader.format())
                        .map(|format| format!("{format:?}").to_ascii_lowercase());
                    self.finish_job(
                        &ctx,
                        &job.id,
                        MediaJobStatus::Completed,
                        Some(100.0),
                        None,
                        Some(
                            json!({"image": {"width": width, "height": height, "format": format}}),
                        ),
                    );
                    self.active.lock().remove(&job.id);
                    return;
                }
                Err(_) => av::probe_file(&ctx, &job.input_path, &active.cancelled, &active.child)
                    .and_then(|metadata| {
                        self.finish_job(
                            &ctx,
                            &job.id,
                            MediaJobStatus::Completed,
                            Some(100.0),
                            None,
                            Some(metadata),
                        );
                        Ok(())
                    }),
            },
        };

        if let Some(partial) = &active.partial {
            if result.is_err() || active.cancelled.load(Ordering::Acquire) {
                output::remove_partial(partial);
            }
        }
        if active.cancelled.load(Ordering::Acquire) {
            self.finish_job(
                &ctx,
                &job.id,
                MediaJobStatus::Cancelled,
                None,
                Some("job cancelled".into()),
                None,
            );
        } else {
            match result {
                Ok(()) => {
                    // Inspect completion is persisted in the match above.
                    if job.operation != "inspect" {
                        self.finish_job(
                            &ctx,
                            &job.id,
                            MediaJobStatus::Completed,
                            Some(100.0),
                            None,
                            None,
                        );
                    }
                }
                Err(error) => self.finish_job(
                    &ctx,
                    &job.id,
                    MediaJobStatus::Failed,
                    None,
                    Some(error.to_string()),
                    None,
                ),
            }
        }
        self.active.lock().remove(&job.id);
        self.prune_history(&ctx);
    }

    fn update_active(&self, job_id: &str, update: impl FnOnce(&mut MediaJob)) {
        let snapshot = {
            let mut active = self.active.lock();
            let Some(current) = active.get_mut(job_id) else {
                return;
            };
            update(&mut current.job);
            current.job.clone()
        };
        self.emit(snapshot);
    }

    fn finish_job(
        &self,
        ctx: &ModuleContext,
        job_id: &str,
        status: MediaJobStatus,
        progress: Option<f64>,
        error: Option<String>,
        metadata: Option<Value>,
    ) {
        self.update_active(job_id, |job| {
            job.status = status;
            if let Some(progress) = progress {
                job.progress = progress;
            }
            job.error = error;
            job.metadata = metadata;
            job.updated_at = chrono::Utc::now().timestamp_millis();
        });
        let snapshot = self
            .active
            .lock()
            .get(job_id)
            .map(|active| active.job.clone());
        if let Some(snapshot) = snapshot {
            if let Err(error) = persist_job(ctx, &snapshot) {
                tracing::error!(%error, job_id, "failed to persist media job");
            }
        }
    }

    fn prune_history(&self, ctx: &ModuleContext) {
        let limit = self.settings(ctx).map(|s| s.history_limit).unwrap_or(200) as i64;
        let result = ctx.db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM media_task_history
                 WHERE status IN ('completed', 'failed', 'cancelled') AND id NOT IN (
                    SELECT id FROM media_task_history
                    WHERE status IN ('completed', 'failed', 'cancelled')
                    ORDER BY created_at DESC LIMIT ?1
                 )",
                params![limit],
            )?;
            Ok(())
        });
        if let Err(error) = result {
            tracing::warn!(%error, "failed to prune media history");
        }
    }

    fn emit(&self, job: MediaJob) {
        if let Some(sink) = self.event_sink.lock().as_ref() {
            sink(job);
        }
    }

    fn job_snapshot(&self, ctx: &ModuleContext, job_id: &str) -> CoreResult<MediaJob> {
        if let Some(active) = self.active.lock().get(job_id) {
            return Ok(active.job.clone());
        }
        ctx.db.with_conn(|conn| {
            conn.query_row(
                "SELECT id, operation, input_path, output_path, status, progress, speed,
                        error, metadata_json, created_at, updated_at
                 FROM media_task_history WHERE id = ?1",
                params![job_id],
                map_job_row,
            )
            .map_err(CoreError::from)
        })
    }
}

impl AppModule for MediaProcModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn name(&self) -> &'static str {
        "基础多媒体处理"
    }

    fn migrations(&self) -> Vec<ModuleMigration> {
        vec![ModuleMigration::new(
            MODULE_ID,
            1,
            "创建多媒体任务历史表",
            "CREATE TABLE IF NOT EXISTS media_task_history (
                id TEXT PRIMARY KEY,
                operation TEXT NOT NULL,
                input_path TEXT NOT NULL,
                output_path TEXT,
                status TEXT NOT NULL,
                progress REAL NOT NULL DEFAULT 0,
                speed TEXT,
                error TEXT,
                metadata_json TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_media_task_history_created
                ON media_task_history(created_at DESC);",
        )]
    }

    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let settings = self.settings(ctx)?;
        self.apply_settings(&settings)?;
        ctx.db.with_conn(|conn| {
            conn.execute(
                "UPDATE media_task_history SET status = 'cancelled',
                    error = 'module stopped before job completion', updated_at = ?1
                 WHERE status IN ('queued', 'running')",
                params![chrono::Utc::now().timestamp_millis()],
            )?;
            Ok(())
        })?;
        *self.runtime.lock() = Some(ctx.clone());
        self.accepting.store(true, Ordering::Release);
        Ok(())
    }

    fn on_shutdown(&self, _ctx: &ModuleContext) -> CoreResult<()> {
        self.accepting.store(false, Ordering::Release);
        let active: Vec<ActiveJob> = self.active.lock().values().cloned().collect();
        for job in active {
            job.cancelled.store(true, Ordering::Release);
            av::kill_child(&job.child);
        }
        self.semaphore.wake_all();
        let workers: Vec<JoinHandle<()>> = self
            .workers
            .lock()
            .drain()
            .map(|(_, worker)| worker)
            .collect();
        for worker in workers {
            let _ = worker.join();
        }
        self.active.lock().clear();
        *self.runtime.lock() = None;
        Ok(())
    }
}

fn validate_batch(input_paths: &[PathBuf]) -> CoreResult<()> {
    if input_paths.is_empty() {
        return Err(CoreError::Config("no input files selected".into()));
    }
    if input_paths.len() > MAX_BATCH_FILES {
        return Err(CoreError::Config(format!(
            "a batch cannot contain more than {MAX_BATCH_FILES} files"
        )));
    }
    for input in input_paths {
        if !input.is_file() {
            return Err(CoreError::Config(format!(
                "input file does not exist: {}",
                input.display()
            )));
        }
    }
    Ok(())
}

fn persist_job(ctx: &ModuleContext, job: &MediaJob) -> CoreResult<()> {
    let metadata = job
        .metadata
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    ctx.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO media_task_history (
                id, operation, input_path, output_path, status, progress, speed, error,
                metadata_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                operation=excluded.operation, input_path=excluded.input_path,
                output_path=excluded.output_path, status=excluded.status,
                progress=excluded.progress, speed=excluded.speed, error=excluded.error,
                metadata_json=excluded.metadata_json, updated_at=excluded.updated_at",
            params![
                job.id,
                job.operation,
                job.input_path.to_string_lossy(),
                job.output_path.as_ref().map(|path| path.to_string_lossy()),
                job.status.as_str(),
                job.progress,
                job.speed,
                job.error,
                metadata,
                job.created_at,
                job.updated_at,
            ],
        )?;
        Ok(())
    })
}

fn map_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MediaJob> {
    let status: String = row.get(4)?;
    let metadata: Option<String> = row.get(8)?;
    Ok(MediaJob {
        id: row.get(0)?,
        operation: row.get(1)?,
        input_path: PathBuf::from(row.get::<_, String>(2)?),
        output_path: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
        status: MediaJobStatus::from_str(&status),
        progress: row.get(5)?,
        speed: row.get(6)?,
        error: row.get(7)?,
        metadata: metadata.and_then(|value| serde_json::from_str(&value).ok()),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
