use super::*;
use crate::{av::ProgressLine, output::prepare_output};
use ::image::{ImageBuffer, Rgb};
use std::fs;
use tempfile::TempDir;

pub(crate) fn test_dir(label: &str) -> TempDir {
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

fn sample_png(path: &Path) {
    let image = ImageBuffer::from_fn(12, 8, |x, y| Rgb([(x * 17) as u8, (y * 29) as u8, 120_u8]));
    image.save(path).unwrap();
}

#[test]
fn config_and_parameters_are_validated() {
    let mut config = MediaProcConfig::default();
    config.max_concurrent_jobs = 0;
    assert!(config.validate().is_err());
    let mut image = ImageConvertOptions::default();
    image.quality = Some(80);
    assert!(image.validate().is_err());
    image.output_format = ImageOutputFormat::Jpeg;
    assert!(image.validate().is_ok());
    image.quality = Some(101);
    assert!(image.validate().is_err());
    image.quality = Some(80);
    image.width = Some(0);
    assert!(image.validate().is_err());
    image.width = None;
    image.preserve_aspect = false;
    assert!(image.validate().is_err());
}

#[test]
fn real_rust_codecs_convert_resize_and_never_overwrite() {
    let dir = test_dir("codecs");
    let input = dir.path().join("图片 with spaces.png");
    sample_png(&input);
    for format in [
        ImageOutputFormat::Png,
        ImageOutputFormat::Jpeg,
        ImageOutputFormat::Webp,
        ImageOutputFormat::Bmp,
        ImageOutputFormat::Ico,
    ] {
        let mut job = MediaJob::new("image_convert", input.clone());
        let (output, partial) =
            prepare_output(&input, Some(dir.path()), format.extension(), &job.id).unwrap();
        job.output_path = Some(output.clone());
        let options = ImageConvertOptions {
            output_format: format,
            quality: (format == ImageOutputFormat::Jpeg).then_some(82),
            width: Some(6),
            height: None,
            preserve_aspect: true,
        };
        image::process(
            &job,
            &options,
            &partial,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        let decoded = ::image::open(&output).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (6, 4));
        assert!(input.exists());
    }
}

#[test]
fn output_names_handle_unicode_spaces_and_conflicts() {
    let dir = test_dir("outputs");
    let input = dir.path().join("示例 文件.png");
    sample_png(&input);
    let (first, _) = prepare_output(&input, None, "webp", "a").unwrap();
    fs::write(&first, b"do not overwrite").unwrap();
    let (second, _) = prepare_output(&input, None, "webp", "b").unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read(&first).unwrap(), b"do not overwrite");
    let (pending_first, _) = prepare_output(&input, None, "bmp", "c").unwrap();
    let (pending_second, _) = prepare_output(&input, None, "bmp", "d").unwrap();
    assert_ne!(pending_first, pending_second);
    assert!(!pending_first.exists());
    crate::output::release_output(&pending_first);
    crate::output::release_output(&pending_second);
}

#[test]
fn ffmpeg_progress_parser_accepts_real_pipe_keys() {
    let mut parser = ProgressLine::default();
    parser.parse("out_time_us=1500000");
    parser.parse("speed= 2.5x");
    parser.parse("progress=continue");
    assert_eq!(parser.out_time_us, Some(1_500_000.0));
    assert_eq!(parser.speed.as_deref(), Some("2.5x"));
    assert!(!parser.ended);
    parser.parse("progress=end");
    assert!(parser.ended);
}

#[test]
fn trim_and_volume_validation_rejects_invalid_ranges() {
    let dir = test_dir("validation");
    let input = dir.path().join("in.wav");
    fs::write(&input, b"placeholder").unwrap();
    let request = |operation| AvJobRequest {
        input_path: input.clone(),
        operation,
        output_format: Some("wav".into()),
        output_dir: None,
        start_seconds: None,
        end_seconds: None,
        volume: None,
        precise: false,
    };
    assert!(request(AvOperation::Trim).validate().is_err());
    let mut trim = request(AvOperation::Trim);
    trim.start_seconds = Some(3.0);
    trim.end_seconds = Some(2.0);
    assert!(trim.validate().is_err());
    let mut volume = request(AvOperation::AdjustVolume);
    volume.volume = Some(-1.0);
    assert!(volume.validate().is_err());
}
