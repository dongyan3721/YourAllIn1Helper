#![cfg(unix)]

use app_core::{AppKernelBuilder, ExternalToolKind};
use mod_media_proc::{AvJobRequest, AvOperation, MediaJobStatus, MediaProcConfig, MediaProcModule};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn test_dir(label: &str) -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".runtime_data")
        .join("mod-media-proc-tests");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("{label}-"))
        .tempdir_in(root)
        .unwrap()
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn wait_stopped(module: &MediaProcModule) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while module.active_job_count() != 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(module.active_job_count(), 0);
}

#[test]
fn cancellation_and_shutdown_kill_children_and_remove_partial_outputs() {
    let dir = test_dir("process-control");
    let input = dir.path().join("输入 video.mp4");
    fs::write(&input, b"test media").unwrap();
    let ffmpeg = dir.path().join("fake ffmpeg");
    let ffprobe = dir.path().join("fake ffprobe");
    executable(
        &ffmpeg,
        "#!/bin/sh\nif [ \"$1\" = \"-version\" ]; then echo 'ffmpeg test'; exit 0; fi\nwhile true; do echo 'out_time_us=1000000'; echo 'speed=1x'; sleep 1; done\n",
    );
    executable(
        &ffprobe,
        "#!/bin/sh\nif [ \"$1\" = \"-version\" ]; then echo 'ffprobe test'; exit 0; fi\necho '{\"format\":{\"duration\":\"3600\"}}'\n",
    );
    let module = Arc::new(MediaProcModule::new());
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .register_module_arc(module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    kernel
        .context()
        .env_probe
        .set_custom_override(ExternalToolKind::Ffmpeg, Some(ffmpeg));
    kernel
        .context()
        .env_probe
        .set_custom_override(ExternalToolKind::Ffprobe, Some(ffprobe));
    let request = || AvJobRequest {
        input_path: input.clone(),
        operation: AvOperation::Convert,
        output_format: Some("mp4".into()),
        output_dir: Some(dir.path().join("out")),
        start_seconds: None,
        end_seconds: None,
        volume: None,
        precise: false,
    };
    let first = module.submit_av_job(kernel.context(), request()).unwrap();
    thread::sleep(Duration::from_millis(250));
    module.cancel_job(kernel.context(), &first.id).unwrap();
    wait_stopped(&module);
    let cancelled = module
        .list_jobs(kernel.context())
        .unwrap()
        .into_iter()
        .find(|job| job.id == first.id)
        .unwrap();
    assert_eq!(cancelled.status, MediaJobStatus::Cancelled);

    let second = module.submit_av_job(kernel.context(), request()).unwrap();
    thread::sleep(Duration::from_millis(250));
    kernel.set_module_enabled("media_proc", false).unwrap();
    assert_eq!(module.active_job_count(), 0);
    assert_eq!(module.worker_count(), 0);
    let stopped = module
        .list_jobs(kernel.context())
        .unwrap()
        .into_iter()
        .find(|job| job.id == second.id)
        .unwrap();
    assert_eq!(stopped.status, MediaJobStatus::Cancelled);
    let partial_count = fs::read_dir(dir.path().join("out"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".partial."))
        .count();
    assert_eq!(partial_count, 0);
}

#[test]
fn semaphore_enforces_configured_concurrency_for_queued_processes() {
    let dir = test_dir("process-concurrency");
    let input = dir.path().join("in.wav");
    fs::write(&input, b"test media").unwrap();
    let ffmpeg = dir.path().join("ffmpeg");
    let ffprobe = dir.path().join("ffprobe");
    let lock = dir.path().join("worker.lock");
    let overflow = dir.path().join("overflow");
    executable(
        &ffmpeg,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"-version\" ]; then echo 'ffmpeg test'; exit 0; fi\nif ! mkdir '{}'; then echo concurrent >> '{}'; fi\nsleep 1\nrmdir '{}'\n",
            lock.display(),
            overflow.display(),
            lock.display(),
        ),
    );
    executable(
        &ffprobe,
        "#!/bin/sh\nif [ \"$1\" = \"-version\" ]; then echo 'ffprobe test'; exit 0; fi\necho '{\"format\":{\"duration\":\"2\"}}'\n",
    );
    let module = Arc::new(MediaProcModule::new());
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .register_module_arc(module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    let settings = MediaProcConfig {
        max_concurrent_jobs: 1,
        ..Default::default()
    };
    kernel
        .update_config(|config| {
            config.modules.settings.insert(
                "media_proc".into(),
                serde_json::to_value(&settings).unwrap(),
            );
        })
        .unwrap();
    module.apply_settings(&settings).unwrap();
    kernel
        .context()
        .env_probe
        .set_custom_override(ExternalToolKind::Ffmpeg, Some(ffmpeg));
    kernel
        .context()
        .env_probe
        .set_custom_override(ExternalToolKind::Ffprobe, Some(ffprobe));
    for _ in 0..3 {
        module
            .submit_av_job(
                kernel.context(),
                AvJobRequest {
                    input_path: input.clone(),
                    operation: AvOperation::Convert,
                    output_format: Some("mp3".into()),
                    output_dir: None,
                    start_seconds: None,
                    end_seconds: None,
                    volume: None,
                    precise: false,
                },
            )
            .unwrap();
    }
    wait_stopped(&module);
    assert!(!overflow.exists(), "configured concurrency was exceeded");
}

#[test]
fn nonzero_ffmpeg_exit_preserves_actionable_failure_reason() {
    let dir = test_dir("process-failure");
    let input = dir.path().join("in.wav");
    fs::write(&input, b"test media").unwrap();
    let ffmpeg = dir.path().join("ffmpeg");
    let ffprobe = dir.path().join("ffprobe");
    executable(
        &ffmpeg,
        "#!/bin/sh\nif [ \"$1\" = \"-version\" ]; then echo 'ffmpeg test'; exit 0; fi\necho 'invalid encoder fixture' >&2\nexit 2\n",
    );
    executable(
        &ffprobe,
        "#!/bin/sh\nif [ \"$1\" = \"-version\" ]; then echo 'ffprobe test'; exit 0; fi\necho '{\"format\":{\"duration\":\"2\"}}'\n",
    );
    let module = Arc::new(MediaProcModule::new());
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .register_module_arc(module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    kernel
        .context()
        .env_probe
        .set_custom_override(ExternalToolKind::Ffmpeg, Some(ffmpeg));
    kernel
        .context()
        .env_probe
        .set_custom_override(ExternalToolKind::Ffprobe, Some(ffprobe));
    let job = module
        .submit_av_job(
            kernel.context(),
            AvJobRequest {
                input_path: input,
                operation: AvOperation::Convert,
                output_format: Some("mp3".into()),
                output_dir: None,
                start_seconds: None,
                end_seconds: None,
                volume: None,
                precise: false,
            },
        )
        .unwrap();
    wait_stopped(&module);
    let failed = module
        .list_jobs(kernel.context())
        .unwrap()
        .into_iter()
        .find(|item| item.id == job.id)
        .unwrap();
    assert_eq!(failed.status, MediaJobStatus::Failed);
    assert!(failed.error.unwrap().contains("invalid encoder fixture"));
}
