use app_core::{AppKernelBuilder, ModuleContext};
use mod_media_proc::{AvJobRequest, AvOperation, MediaJob, MediaJobStatus, MediaProcModule};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn test_dir() -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".runtime_data")
        .join("mod-media-proc-tests");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("real-ffmpeg-")
        .tempdir_in(root)
        .unwrap()
}

fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn generate_input(ffmpeg: &Path, input: &Path) {
    let output = Command::new(ffmpeg)
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-n",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:sample_rate=44100",
            "-t",
            "2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
        ])
        .arg(input)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "sample generation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn wait_for_job(module: &MediaProcModule, ctx: &ModuleContext, id: &str) -> MediaJob {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let job = module
            .list_jobs(ctx)
            .unwrap()
            .into_iter()
            .find(|job| job.id == id)
            .unwrap();
        if job.status != MediaJobStatus::Queued && job.status != MediaJobStatus::Running {
            assert_eq!(module.active_job_count(), 0);
            return job;
        }
        assert!(Instant::now() < deadline, "media job {id} timed out");
        thread::sleep(Duration::from_millis(25));
    }
}

fn probe(ffprobe: &Path, input: &Path) -> Value {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(input)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "output is not readable by FFprobe: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn stream_types(metadata: &Value) -> Vec<&str> {
    metadata["streams"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|stream| stream["codec_type"].as_str())
        .collect()
}

fn duration(metadata: &Value) -> f64 {
    metadata["format"]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

fn request(input: &Path, output_dir: &Path, operation: AvOperation, format: &str) -> AvJobRequest {
    AvJobRequest {
        input_path: input.to_path_buf(),
        operation,
        output_format: Some(format.into()),
        output_dir: Some(output_dir.to_path_buf()),
        start_seconds: None,
        end_seconds: None,
        volume: None,
        precise: false,
    }
}

#[test]
fn installed_ffmpeg_executes_all_media_operations_and_produces_probeable_outputs() {
    if !tool_available("ffmpeg") || !tool_available("ffprobe") {
        eprintln!("FFmpeg/FFprobe not installed; real runtime integration test not executed");
        return;
    }
    let ffmpeg = PathBuf::from("ffmpeg");
    let ffprobe = PathBuf::from("ffprobe");
    let dir = test_dir();
    let input = dir.path().join("无版权 样本.mp4");
    let output_dir = dir.path().join("输出");
    generate_input(&ffmpeg, &input);

    let module = Arc::new(MediaProcModule::new());
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .register_module_arc(module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    let environment = module.check_environment(kernel.context(), true);
    assert_eq!(environment["ffmpeg"]["available"], true);
    assert_eq!(environment["ffprobe"]["available"], true);

    let events = Arc::new(Mutex::new(Vec::<MediaJob>::new()));
    let event_copy = events.clone();
    module.set_event_sink(Some(Arc::new(move |job| {
        event_copy.lock().unwrap().push(job);
    })));

    let inspection = module
        .inspect_file(kernel.context(), input.clone())
        .unwrap();
    let inspection = wait_for_job(&module, kernel.context(), &inspection.id);
    assert_eq!(inspection.status, MediaJobStatus::Completed);
    let metadata = inspection.metadata.unwrap();
    assert!(stream_types(&metadata).contains(&"video"));
    assert!(stream_types(&metadata).contains(&"audio"));
    assert!((duration(&metadata) - 2.0).abs() < 0.2);

    let convert = module
        .submit_av_job(
            kernel.context(),
            request(&input, &output_dir, AvOperation::Convert, "mkv"),
        )
        .unwrap();
    let convert = wait_for_job(&module, kernel.context(), &convert.id);
    assert_eq!(
        convert.status,
        MediaJobStatus::Completed,
        "{:?}",
        convert.error
    );
    let converted = probe(&ffprobe, convert.output_path.as_ref().unwrap());
    assert!(stream_types(&converted).contains(&"video"));
    assert!(stream_types(&converted).contains(&"audio"));

    for precise in [false, true] {
        let mut trim = request(&input, &output_dir, AvOperation::Trim, "mp4");
        trim.start_seconds = Some(0.25);
        trim.end_seconds = Some(1.25);
        trim.precise = precise;
        let job = module.submit_av_job(kernel.context(), trim).unwrap();
        let job = wait_for_job(&module, kernel.context(), &job.id);
        assert_eq!(job.status, MediaJobStatus::Completed, "{:?}", job.error);
        let trimmed = probe(&ffprobe, job.output_path.as_ref().unwrap());
        assert!(duration(&trimmed) > 0.0);
        if precise {
            assert!((duration(&trimmed) - 1.0).abs() < 0.25);
        }
    }

    let extracted = module
        .submit_av_job(
            kernel.context(),
            request(&input, &output_dir, AvOperation::ExtractAudio, "mp3"),
        )
        .unwrap();
    let extracted = wait_for_job(&module, kernel.context(), &extracted.id);
    assert_eq!(
        extracted.status,
        MediaJobStatus::Completed,
        "{:?}",
        extracted.error
    );
    let audio = probe(&ffprobe, extracted.output_path.as_ref().unwrap());
    assert_eq!(stream_types(&audio), vec!["audio"]);

    let mut adjusted = request(&input, &output_dir, AvOperation::AdjustVolume, "m4a");
    adjusted.volume = Some(0.25);
    let adjusted = module.submit_av_job(kernel.context(), adjusted).unwrap();
    let adjusted = wait_for_job(&module, kernel.context(), &adjusted.id);
    assert_eq!(
        adjusted.status,
        MediaJobStatus::Completed,
        "{:?}",
        adjusted.error
    );
    let quieter = probe(&ffprobe, adjusted.output_path.as_ref().unwrap());
    assert_eq!(stream_types(&quieter), vec!["audio"]);

    let snapshots = events.lock().unwrap();
    assert!(snapshots.iter().any(|job| job.progress > 0.0));
    assert!(snapshots
        .iter()
        .all(|job| (0.0..=100.0).contains(&job.progress)));
    assert!(snapshots.iter().any(|job| job.speed.is_some()));
    assert_eq!(
        fs::read_dir(output_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".partial."))
            .count(),
        0
    );
}
