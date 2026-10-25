//! GIF 模块单元测试。
//!
//! 验证：
//! 1. 分辨率预设换算保持宽高比与偶数规范化；
//! 2. 真实编码并通过 `gif` 解码器检验帧数、尺寸与帧间隔；
//! 3. 录制完成独占落盘保存；
//! 4. 取消删除半成品，不留残留垃圾文件；
//! 5. 停用等待后台工作线程安全退出。

use crate::gif::{
    calculate_target_dimensions,
    encoder::GifStreamEncoder,
    options::{GifFramerate, GifRecordingConfig, GifResolutionPreset},
    recorder::GifRecordingSession,
};
use os_hal::{CapturedFrame, DisplayMonitorInfo, HalResult, ScreenCapturer};
use std::io::Cursor;
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn test_gif_resolution_presets() {
    // 1. 原画尺寸保持
    let (w, h) = calculate_target_dimensions(1920, 1080, &GifResolutionPreset::Original);
    assert_eq!((w, h), (1920, 1080));

    // 2. 4K 降采样到 1080p
    let (w, h) = calculate_target_dimensions(3840, 2160, &GifResolutionPreset::P1080);
    assert_eq!((w, h), (1920, 1080));

    // 3. 1080p 降采样到 720p
    let (w, h) = calculate_target_dimensions(1920, 1080, &GifResolutionPreset::P720);
    assert_eq!((w, h), (1280, 720));

    // 4. 1080p 降采样到 480p
    let (w, h) = calculate_target_dimensions(1920, 1080, &GifResolutionPreset::P480);
    assert_eq!((w, h), (854, 480));

    // 5. 奇数宽高规范化为偶数
    let (w, h) = calculate_target_dimensions(101, 203, &GifResolutionPreset::Original);
    assert_eq!((w, h), (102, 204));
    assert_eq!(w % 2, 0);
    assert_eq!(h % 2, 0);

    // 6. 自定义分辨率
    let (w, h) = calculate_target_dimensions(
        1920,
        1080,
        &GifResolutionPreset::Custom {
            width: 320,
            height: 240,
        },
    );
    assert_eq!((w, h), (320, 240));
}

#[test]
fn test_gif_encoder_round_trip_and_decode_validation() {
    let target_w = 80;
    let target_h = 60;
    let fps = GifFramerate::Fps15;

    let mut output_buf = Vec::new();
    let mut encoder = GifStreamEncoder::new(&mut output_buf, target_w, target_h, fps, 10).unwrap();

    // 构造 5 帧具有不同色块的画面
    for frame_idx in 0..5 {
        let mut rgba = Vec::with_capacity((target_w * target_h * 4) as usize);
        for y in 0..target_h {
            for x in 0..target_w {
                let r = ((x * 3 + frame_idx * 40) % 256) as u8;
                let g = ((y * 4 + frame_idx * 20) % 256) as u8;
                let b = ((x + y + frame_idx * 50) % 256) as u8;
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        encoder.add_frame(&rgba, target_w, target_h).unwrap();
    }

    let encoded_count = encoder.finish().unwrap();
    assert_eq!(encoded_count, 5);
    assert!(!output_buf.is_empty());

    // 使用官方 `gif` crate 解码器读取并逐项验证！
    let decode_options = gif::DecodeOptions::new();
    let mut reader = decode_options
        .read_info(Cursor::new(&output_buf))
        .expect("Generated GIF must be decodable");

    // 验证头信息
    assert_eq!(reader.width() as u32, target_w);
    assert_eq!(reader.height() as u32, target_h);

    // 逐帧解码验证
    let mut decoded_frame_count = 0;
    while let Some(frame) = reader.read_next_frame().expect("Frame must decode cleanly") {
        assert_eq!(frame.width as u32, target_w);
        assert_eq!(frame.height as u32, target_h);
        // 15 FPS 对应 7 centiseconds (70ms)
        assert_eq!(frame.delay, fps.delay_centisecs());
        decoded_frame_count += 1;
    }

    assert_eq!(decoded_frame_count, 5);
}

/// 录制测试 Mock 捕获器。
struct MockGifCapturer {
    counter: parking_lot::Mutex<u8>,
}

impl ScreenCapturer for MockGifCapturer {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        Ok(vec![])
    }
    fn capture_monitor(&self, _id: Option<u32>) -> HalResult<CapturedFrame> {
        unimplemented!()
    }
    fn capture_region(&self, bounds: app_core::SelectionBox) -> HalResult<CapturedFrame> {
        let mut c = self.counter.lock();
        *c = c.wrapping_add(1);
        let val = *c;

        let mut rgba = Vec::with_capacity((bounds.width * bounds.height * 4) as usize);
        for _ in 0..(bounds.width * bounds.height) {
            rgba.extend_from_slice(&[val, val.wrapping_mul(2), val.wrapping_mul(3), 255]);
        }

        Ok(CapturedFrame {
            width: bounds.width,
            height: bounds.height,
            scale_factor_milli: 1000,
            rgba_bytes: rgba,
        })
    }
}

#[test]
fn test_gif_recording_session_completion_and_file_creation() {
    let dir = tempdir().unwrap();
    let save_dir = dir.path().to_path_buf();

    let capturer = Arc::new(MockGifCapturer {
        counter: parking_lot::Mutex::new(0),
    });

    let config = GifRecordingConfig {
        resolution_preset: GifResolutionPreset::Original,
        framerate: GifFramerate::Fps15,
        max_duration_secs: 10,
        max_frames: 100,
        sample_factor: 10,
    };

    let session = GifRecordingSession::new(
        "test_gif_rec".to_string(),
        app_core::SelectionBox {
            x: 0,
            y: 0,
            width: 64,
            height: 48,
        },
        save_dir.clone(),
        "Screenshot".to_string(),
        config,
        capturer,
        None,
    )
    .expect("Session create");

    // 录制约 200ms（产生约 3 帧）
    std::thread::sleep(std::time::Duration::from_millis(200));

    assert!(session.stop());
    session.wait_finished(std::time::Duration::from_secs(2));

    let st = session.status();
    assert!(!st.is_recording);
    assert!(st.saved_path.is_some(), "Saved path must exist: {:?}", st);

    let saved_path = std::path::PathBuf::from(st.saved_path.unwrap());
    assert!(saved_path.exists());
    assert!(saved_path.extension().unwrap() == "gif");

    // 验证生成的实际落盘文件可被真实解码
    let bytes = std::fs::read(&saved_path).unwrap();
    let mut reader = gif::DecodeOptions::new()
        .read_info(Cursor::new(&bytes))
        .expect("Saved GIF file must be valid and decodable");
    assert_eq!(reader.width(), 64);
    assert_eq!(reader.height(), 48);

    let mut decoded = 0;
    while let Some(_) = reader.read_next_frame().unwrap() {
        decoded += 1;
    }
    assert!(decoded >= 1);
}

#[test]
fn test_gif_recording_session_cancellation_deletes_temp_file() {
    let dir = tempdir().unwrap();
    let save_dir = dir.path().to_path_buf();

    let capturer = Arc::new(MockGifCapturer {
        counter: parking_lot::Mutex::new(0),
    });

    let session = GifRecordingSession::new(
        "test_gif_cancel".to_string(),
        app_core::SelectionBox {
            x: 0,
            y: 0,
            width: 40,
            height: 40,
        },
        save_dir.clone(),
        "Screenshot".to_string(),
        GifRecordingConfig::default(),
        capturer,
        None,
    )
    .expect("Session create");

    std::thread::sleep(std::time::Duration::from_millis(150));

    // 用户在录制中取消
    assert!(session.cancel());
    session.wait_finished(std::time::Duration::from_secs(2));

    let st = session.status();
    assert!(!st.is_recording);
    assert!(st.saved_path.is_none());

    // 确认目录下没有任何残留的 .gif 或临时文件
    let entries: Vec<_> = std::fs::read_dir(&save_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        entries.is_empty(),
        "No temp files should remain after cancel: {:?}",
        entries
    );
}
