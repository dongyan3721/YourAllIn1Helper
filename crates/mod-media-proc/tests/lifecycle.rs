use app_core::{AppKernelBuilder, CoreError};
use mod_media_proc::{ImageConvertOptions, ImageOutputFormat, MediaJobStatus, MediaProcModule};
use std::{fs, path::PathBuf, sync::Arc, thread, time::Duration, time::Instant};

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

fn wait_for_jobs(module: &MediaProcModule) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while module.active_job_count() > 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(module.active_job_count(), 0, "media jobs did not finish");
}

#[test]
fn image_jobs_survive_restart_and_module_can_restart_cleanly() {
    let dir = test_dir("lifecycle");
    let input = dir.path().join("输入 图.png");
    image::RgbImage::new(9, 7).save(&input).unwrap();
    let module = Arc::new(MediaProcModule::new());
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(false)
        .register_module_arc(module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    let jobs = module
        .submit_image_jobs(
            kernel.context(),
            vec![input],
            Some(dir.path().join("输出 目录")),
            ImageConvertOptions {
                output_format: ImageOutputFormat::Webp,
                width: Some(3),
                height: None,
                ..Default::default()
            },
        )
        .unwrap();
    wait_for_jobs(&module);
    let records = module.list_jobs(kernel.context()).unwrap();
    assert_eq!(records[0].status, MediaJobStatus::Completed);
    assert!(records[0].output_path.as_ref().unwrap().exists());
    assert_eq!(jobs.len(), 1);

    kernel.set_module_enabled("media_proc", false).unwrap();
    assert_eq!(module.worker_count(), 0);
    assert!(matches!(
        module.list_jobs(kernel.context()),
        Ok(records) if !records.is_empty()
    ));
    kernel.set_module_enabled("media_proc", true).unwrap();
    kernel.set_module_enabled("media_proc", false).unwrap();
    drop(kernel);

    let restarted = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(false)
        .register_module(MediaProcModule::new())
        .unwrap()
        .bootstrap()
        .unwrap();
    let history = restarted.context().db.with_conn(|conn| {
        conn.query_row("SELECT COUNT(*) FROM media_task_history", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(CoreError::from)
    });
    assert_eq!(history.unwrap(), 1);
}
