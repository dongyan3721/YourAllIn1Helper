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
use os_hal::{CapturedFrame, DisplayMonitorInfo, HalResult, ScreenCapturer, WindowHandleId};
use std::io::Cursor;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
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

    let selection = app_core::SelectionBox {
        x: 0,
        y: 0,
        width: 64,
        height: 48,
    };
    let session = GifRecordingSession::new(
        "test_gif_rec".to_string(),
        selection,
        selection,
        save_dir.clone(),
        "Screenshot".to_string(),
        config,
        capturer,
        None,
        None,
        0,
    )
    .expect("Session create");

    // 录制约 200ms（产生约 3 帧）
    std::thread::sleep(std::time::Duration::from_millis(200));

    assert!(session.stop());
    session.wait_finished(std::time::Duration::from_secs(2));

    let st = session.status();
    assert!(!st.is_recording);
    assert!(
        st.pending_path.is_some(),
        "Pending GIF must exist: {:?}",
        st
    );
    assert!(st.saved_path.is_none());
    let saved_path = session.commit().expect("confirm save");
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

    let selection = app_core::SelectionBox {
        x: 0,
        y: 0,
        width: 40,
        height: 40,
    };
    let session = GifRecordingSession::new(
        "test_gif_cancel".to_string(),
        selection,
        selection,
        save_dir.clone(),
        "Screenshot".to_string(),
        GifRecordingConfig::default(),
        capturer,
        None,
        None,
        0,
    )
    .expect("Session create");

    std::thread::sleep(std::time::Duration::from_millis(150));

    // 用户在录制中取消
    assert!(session.cancel());
    session.wait_finished(std::time::Duration::from_secs(2));
    session.discard_pending();

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

/// 记录排除标识是否真正传到抓屏端，并返回单一颜色的帧。
struct ExclusionAwareCapturer {
    seen_exclusion: AtomicBool,
    rgba: [u8; 4],
}

impl ScreenCapturer for ExclusionAwareCapturer {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        Ok(vec![])
    }
    fn capture_monitor(&self, _id: Option<u32>) -> HalResult<CapturedFrame> {
        unimplemented!()
    }
    fn capture_region(&self, bounds: app_core::SelectionBox) -> HalResult<CapturedFrame> {
        let mut pixels = Vec::with_capacity((bounds.width * bounds.height * 4) as usize);
        for _ in 0..(bounds.width * bounds.height) {
            pixels.extend_from_slice(&self.rgba);
        }
        Ok(CapturedFrame {
            width: bounds.width,
            height: bounds.height,
            scale_factor_milli: 1000,
            rgba_bytes: pixels,
        })
    }
    fn capture_region_excluding(
        &self,
        bounds: app_core::SelectionBox,
        exclusion: Option<WindowHandleId>,
    ) -> HalResult<CapturedFrame> {
        self.seen_exclusion
            .store(exclusion.is_some(), Ordering::SeqCst);
        self.capture_region(bounds)
    }
}

fn solid_overlay_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba(rgba));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("overlay PNG encode");
    bytes
}

fn first_frame_rgb(bytes: &[u8]) -> (u8, u8, u8) {
    let mut reader = gif::DecodeOptions::new()
        .read_info(Cursor::new(bytes))
        .expect("GIF must decode");
    let frame = reader
        .read_next_frame()
        .expect("read first frame")
        .expect("GIF must contain a frame");
    let palette = frame.palette.as_ref().expect("frame palette");
    let index = frame.buffer[0] as usize * 3;
    (palette[index], palette[index + 1], palette[index + 2])
}

fn recording_config() -> GifRecordingConfig {
    GifRecordingConfig {
        resolution_preset: GifResolutionPreset::Original,
        framerate: GifFramerate::Fps30,
        max_duration_secs: 5,
        max_frames: 2,
        sample_factor: 1,
    }
}

#[test]
fn test_gif_recording_blends_ink_when_capture_excludes_overlay() {
    let dir = tempdir().unwrap();
    let capturer = Arc::new(ExclusionAwareCapturer {
        seen_exclusion: AtomicBool::new(false),
        // 抓到的真实画面是黑的；只有重新合成红色墨迹后才应呈现红色。
        rgba: [0, 0, 0, 255],
    });
    let overlay = solid_overlay_png(8, 8, [255, 0, 0, 255]);
    let selection = app_core::SelectionBox {
        x: 0,
        y: 0,
        width: 8,
        height: 8,
    };
    let session = GifRecordingSession::new(
        "test_gif_exclusion".to_string(),
        selection,
        selection,
        dir.path().to_path_buf(),
        "Screenshot".to_string(),
        recording_config(),
        capturer.clone(),
        None,
        Some(overlay),
        99,
    )
    .expect("Session create");

    std::thread::sleep(std::time::Duration::from_millis(250));
    session.stop();
    session.wait_finished(std::time::Duration::from_secs(2));

    assert!(
        capturer.seen_exclusion.load(Ordering::SeqCst),
        "recorder must forward the overlay exclusion to the capturer"
    );
    let status = session.status();
    let pending = status.pending_path.expect("pending GIF must exist");
    let bytes = std::fs::read(pending).unwrap();
    let (r, g, b) = first_frame_rgb(&bytes);
    assert!(
        r > 150 && g < 80 && b < 80,
        "excluded overlay annotations must be re-blended once, got rgb({r}, {g}, {b})"
    );
}

#[test]
fn test_gif_recording_keeps_capture_when_overlay_is_not_excluded() {
    let dir = tempdir().unwrap();
    let capturer = Arc::new(ExclusionAwareCapturer {
        seen_exclusion: AtomicBool::new(false),
        rgba: [0, 0, 255, 255],
    });
    let overlay = solid_overlay_png(8, 8, [255, 0, 0, 255]);
    let selection = app_core::SelectionBox {
        x: 0,
        y: 0,
        width: 8,
        height: 8,
    };
    let session = GifRecordingSession::new(
        "test_gif_no_exclusion".to_string(),
        selection,
        selection,
        dir.path().to_path_buf(),
        "Screenshot".to_string(),
        recording_config(),
        capturer.clone(),
        None,
        Some(overlay),
        0,
    )
    .expect("Session create");

    std::thread::sleep(std::time::Duration::from_millis(250));
    session.stop();
    session.wait_finished(std::time::Duration::from_secs(2));

    assert!(
        !capturer.seen_exclusion.load(Ordering::SeqCst),
        "no exclusion must be registered for a normal capture"
    );
    let status = session.status();
    let pending = status.pending_path.expect("pending GIF must exist");
    let bytes = std::fs::read(pending).unwrap();
    let (r, g, b) = first_frame_rgb(&bytes);
    assert!(
        b > 150 && r < 80 && g < 80,
        "capture already contains the overlay, so ink must not be blended twice; got rgb({r}, {g}, {b})"
    );
}

#[test]
fn test_gif_encoder_preserves_non_black_content() {
    let mut output = Vec::new();
    let mut encoder = GifStreamEncoder::new(&mut output, 8, 8, GifFramerate::Fps30, 1).unwrap();
    let white = vec![255u8; 8 * 8 * 4];
    encoder.add_frame(&white, 8, 8).unwrap();
    encoder.finish().unwrap();

    let (r, g, b) = first_frame_rgb(&output);
    assert!(
        r > 200 && g > 200 && b > 200,
        "bright RGBA input must not decay to a black GIF frame, got rgb({r}, {g}, {b})"
    );
}

/// 单测注入抓屏失败；不依赖真实桌面或系统权限。
struct FailingCapturer;

impl ScreenCapturer for FailingCapturer {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        Ok(vec![])
    }

    fn capture_monitor(&self, _id: Option<u32>) -> HalResult<CapturedFrame> {
        Err(os_hal::HalError::NativeCallFailed(
            "native capture failed".into(),
        ))
    }

    fn capture_region(&self, _bounds: app_core::SelectionBox) -> HalResult<CapturedFrame> {
        self.capture_monitor(None)
    }
}

#[test]
fn test_gif_native_capture_failure_stops_and_removes_partial_file() {
    let dir = tempdir().unwrap();
    let selection = app_core::SelectionBox {
        x: 0,
        y: 0,
        width: 8,
        height: 8,
    };
    let session = GifRecordingSession::new(
        "failed-capture".into(),
        selection,
        selection,
        dir.path().to_path_buf(),
        "Screenshot".into(),
        recording_config(),
        Arc::new(FailingCapturer),
        None,
        None,
        0,
    )
    .unwrap();
    // 等待实际工作线程自行结束，测试不主动停止来掩盖无限重试。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while session.status().is_recording && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let status = session.status();
    session.wait_finished(std::time::Duration::from_secs(2));
    assert!(!status.is_recording);
    assert!(status.error.unwrap().contains("native capture failed"));
    assert!(status.pending_path.is_none());
    assert!(status.saved_path.is_none());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

fn tiny_selection() -> app_core::SelectionBox {
    app_core::SelectionBox {
        x: 0,
        y: 0,
        width: 8,
        height: 8,
    }
}

fn solid_frame(bounds: app_core::SelectionBox) -> CapturedFrame {
    let pixels = (bounds.width * bounds.height) as usize;
    CapturedFrame {
        width: bounds.width,
        height: bounds.height,
        scale_factor_milli: 1000,
        rgba_bytes: vec![20, 40, 60, 255].repeat(pixels),
    }
}

fn wait_recording_ended(session: &GifRecordingSession) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while session.status().is_recording && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    session.wait_finished(std::time::Duration::from_secs(2));
}

/// 登记句柄可在首帧之后被测试改写。抓屏始终返回成功，用来证明录制循环自己停下来。
struct RegistryCapturer {
    registered: parking_lot::Mutex<Option<WindowHandleId>>,
    seen: parking_lot::Mutex<Vec<Option<WindowHandleId>>>,
    /// `Some(next)` 在第一次抓取返回前替换登记；`None` 表示保持不变。
    replace_after_first: Option<Option<WindowHandleId>>,
}

impl RegistryCapturer {
    fn new(
        initial: Option<WindowHandleId>,
        replace_after_first: Option<Option<WindowHandleId>>,
    ) -> Self {
        Self {
            registered: parking_lot::Mutex::new(initial),
            seen: parking_lot::Mutex::new(Vec::new()),
            replace_after_first,
        }
    }
}

impl ScreenCapturer for RegistryCapturer {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        Ok(vec![])
    }

    fn capture_monitor(&self, _id: Option<u32>) -> HalResult<CapturedFrame> {
        Err(os_hal::HalError::NativeCallFailed(
            "monitor capture is not used".into(),
        ))
    }

    fn capture_region(&self, bounds: app_core::SelectionBox) -> HalResult<CapturedFrame> {
        Ok(solid_frame(bounds))
    }

    fn active_capture_exclusion(&self) -> Option<WindowHandleId> {
        *self.registered.lock()
    }

    fn capture_region_excluding(
        &self,
        bounds: app_core::SelectionBox,
        exclusion: Option<WindowHandleId>,
    ) -> HalResult<CapturedFrame> {
        let first = {
            let mut seen = self.seen.lock();
            let first = seen.is_empty();
            seen.push(exclusion);
            first
        };
        if first {
            if let Some(next) = self.replace_after_first {
                *self.registered.lock() = next;
            }
        }
        // 第二次仍返回成功帧。循环若沿用旧句柄，就会留下可预览的 GIF。
        Ok(solid_frame(bounds))
    }
}

fn start_registry_gif(
    capturer: Arc<RegistryCapturer>,
    exclude_window_id: u32,
    session_id: &str,
) -> (tempfile::TempDir, Arc<GifRecordingSession>) {
    let dir = tempdir().unwrap();
    let selection = tiny_selection();
    let session = GifRecordingSession::new(
        session_id.to_string(),
        selection,
        selection,
        dir.path().to_path_buf(),
        "Screenshot".to_string(),
        recording_config(),
        capturer,
        None,
        None,
        exclude_window_id,
    )
    .expect("session create");
    (dir, session)
}

#[test]
fn test_gif_recording_accepts_stable_registered_handle() {
    let capturer = Arc::new(RegistryCapturer::new(Some(5), None));
    let (_dir, session) = start_registry_gif(capturer.clone(), 0, "stable-handle");
    wait_recording_ended(&session);
    let status = session.status();
    assert!(
        status.error.is_none(),
        "stable handle must keep recording: {status:?}"
    );
    assert!(status.pending_path.is_some());
    assert_eq!(capturer.seen.lock().as_slice(), &[Some(5), Some(5)]);
}

#[test]
fn test_gif_recording_stops_when_registered_handle_changes() {
    let capturer = Arc::new(RegistryCapturer::new(Some(11), Some(Some(22))));
    let (dir, session) = start_registry_gif(capturer.clone(), 0, "changed-handle");
    wait_recording_ended(&session);
    let status = session.status();
    assert!(!status.is_recording);
    let error = status.error.expect("handle change must fail the recording");
    assert!(error.contains("句柄已更换"), "unexpected error: {error}");
    assert!(status.pending_path.is_none());
    assert!(status.saved_path.is_none());
    assert_eq!(capturer.seen.lock().as_slice(), &[Some(11)]);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn test_gif_recording_stops_when_registered_handle_disappears() {
    let capturer = Arc::new(RegistryCapturer::new(Some(11), Some(None)));
    let (dir, session) = start_registry_gif(capturer.clone(), 0, "lost-handle");
    wait_recording_ended(&session);
    let status = session.status();
    assert!(!status.is_recording);
    let error = status.error.expect("lost handle must fail the recording");
    assert!(error.contains("排除已失效"), "unexpected error: {error}");
    assert!(status.pending_path.is_none());
    assert!(status.saved_path.is_none());
    assert_eq!(capturer.seen.lock().as_slice(), &[Some(11)]);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn test_gif_recording_keeps_explicit_window_id_when_registry_changes() {
    let capturer = Arc::new(RegistryCapturer::new(Some(11), Some(None)));
    let (_dir, session) = start_registry_gif(capturer.clone(), 99, "explicit-window-id");
    wait_recording_ended(&session);
    let status = session.status();
    assert!(
        status.error.is_none(),
        "explicit CGWindowID must ignore the Windows registry: {status:?}"
    );
    assert!(status.pending_path.is_some());
    assert_eq!(capturer.seen.lock().as_slice(), &[Some(99), Some(99)]);
}

/// 不覆盖 `capture_region_excluding`，专门走默认实现。
struct OverflowRegistryCapturer {
    region_calls: AtomicUsize,
}

impl ScreenCapturer for OverflowRegistryCapturer {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        Ok(vec![])
    }

    fn capture_monitor(&self, _id: Option<u32>) -> HalResult<CapturedFrame> {
        Err(os_hal::HalError::NativeCallFailed(
            "monitor capture is not used".into(),
        ))
    }

    fn capture_region(&self, bounds: app_core::SelectionBox) -> HalResult<CapturedFrame> {
        self.region_calls.fetch_add(1, Ordering::SeqCst);
        Ok(solid_frame(bounds))
    }

    fn active_capture_exclusion(&self) -> Option<WindowHandleId> {
        Some(0x0000_7FFE_1234_5678)
    }
}

#[test]
fn test_gif_recording_rejects_overflow_exclusion_without_partial_file() {
    let dir = tempdir().unwrap();
    let capturer = Arc::new(OverflowRegistryCapturer {
        region_calls: AtomicUsize::new(0),
    });
    let selection = tiny_selection();
    let session = GifRecordingSession::new(
        "overflow-handle".into(),
        selection,
        selection,
        dir.path().to_path_buf(),
        "Screenshot".into(),
        recording_config(),
        capturer.clone(),
        None,
        None,
        0,
    )
    .unwrap();
    wait_recording_ended(&session);
    let status = session.status();
    assert!(!status.is_recording);
    let error = status
        .error
        .expect("overflow handle must fail the recording");
    assert!(
        error.contains("32 位"),
        "default exclusion must reject the handle, got {error}"
    );
    assert_eq!(capturer.region_calls.load(Ordering::SeqCst), 0);
    assert!(status.pending_path.is_none());
    assert!(status.saved_path.is_none());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
