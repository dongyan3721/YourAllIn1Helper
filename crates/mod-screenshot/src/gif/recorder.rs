//! GIF 后台录制会话管理。
//!
//! 在后台线程定时抓取选区并流式编码，统计录制时长与帧数，
//! 支持防覆盖独占落盘保存、取消删除半成品与停用优雅等待退出。

use super::{
    encoder::GifStreamEncoder,
    options::{calculate_target_dimensions, GifRecordingConfig},
};
use crate::export::timestamped_stem;
use app_core::SelectionBox;
use os_hal::ScreenCapturer;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tracing::{info, warn};

/// GIF 录制状态快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GifRecordingStatus {
    pub is_recording: bool,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub frame_count: usize,
    pub file_size_bytes: u64,
    /// 确认保留之前的临时 GIF。预览读它，确认后才写成最终文件。
    pub pending_path: Option<String>,
    pub saved_path: Option<String>,
    /// 到达时长或帧数上限后自行停止。
    pub auto_stopped: bool,
    pub error: Option<String>,
}

/// 录制统计事件载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GifRecordingStatsPayload {
    pub duration_ms: u64,
    pub frame_count: usize,
    pub file_size_bytes: u64,
}

pub type GifEventSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// GIF 录制会话。
pub struct GifRecordingSession {
    session_id: String,
    selection: SelectionBox,
    save_dir: PathBuf,
    file_prefix: String,
    status: Arc<RwLock<GifRecordingStatus>>,
    stop_signal: Arc<AtomicBool>,
    cancel_signal: Arc<AtomicBool>,
    worker_handle: Mutex<Option<JoinHandle<()>>>,
}

impl GifRecordingSession {
    pub fn new(
        session_id: String,
        selection: SelectionBox,
        capture_bounds: SelectionBox,
        save_dir: PathBuf,
        file_prefix: String,
        config: GifRecordingConfig,
        capturer: Arc<dyn ScreenCapturer>,
        event_sink: Option<GifEventSink>,
        overlay_png: Option<Vec<u8>>,
        exclude_window_id: u32,
    ) -> Result<Arc<Self>, anyhow::Error> {
        let (target_w, target_h) = calculate_target_dimensions(
            selection.width,
            selection.height,
            &config.resolution_preset,
        );

        let stop_signal = Arc::new(AtomicBool::new(false));
        let cancel_signal = Arc::new(AtomicBool::new(false));

        let status = Arc::new(RwLock::new(GifRecordingStatus {
            is_recording: true,
            width: target_w,
            height: target_h,
            duration_ms: 0,
            frame_count: 0,
            file_size_bytes: 0,
            pending_path: None,
            saved_path: None,
            auto_stopped: false,
            error: None,
        }));

        let session = Arc::new(Self {
            session_id: session_id.clone(),
            selection,
            save_dir: save_dir.clone(),
            file_prefix: file_prefix.clone(),
            status: status.clone(),
            stop_signal: stop_signal.clone(),
            cancel_signal: cancel_signal.clone(),
            worker_handle: Mutex::new(None),
        });

        // 生成临时文件路径
        let temp_file_path = save_dir.join(format!(".tmp_gif_{}.gif", session_id));

        let session_clone = session.clone();
        let handle = thread::Builder::new()
            .name("aio-screenshot-gif".to_string())
            .spawn(move || {
                Self::run_recording_loop(
                    session_id,
                    capture_bounds,
                    target_w,
                    target_h,
                    config,
                    temp_file_path,
                    capturer,
                    overlay_png,
                    exclude_window_id,
                    status,
                    stop_signal,
                    cancel_signal,
                    event_sink,
                );
            })?;

        *session_clone.worker_handle.lock() = Some(handle);
        Ok(session)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn selection(&self) -> SelectionBox {
        self.selection
    }

    pub fn status(&self) -> GifRecordingStatus {
        self.status.read().clone()
    }

    /// 停止录制并生成最终文件。
    pub fn stop(&self) -> bool {
        if !self.stop_signal.swap(true, Ordering::SeqCst) {
            info!(session_id = %self.session_id, "GIF recording stop signal sent");
            true
        } else {
            false
        }
    }

    /// 取消录制并清理临时文件。
    pub fn cancel(&self) -> bool {
        self.cancel_signal.store(true, Ordering::SeqCst);
        self.stop_signal.store(true, Ordering::SeqCst);
        info!(session_id = %self.session_id, "GIF recording cancel signal sent");
        true
    }

    /// 删除已经写好、但用户放弃的临时 GIF。工作线程退出后再调用，避免和收尾抢文件。
    pub fn discard_pending(&self) {
        let path = self.status.write().pending_path.take();
        if let Some(path) = path {
            let _ = fs::remove_file(path);
        }
    }

    /// 把临时 GIF 独占写成最终文件。确认保留时调用。
    pub fn commit(&self) -> Result<PathBuf, anyhow::Error> {
        let pending = self
            .status
            .read()
            .pending_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("没有可保存的 GIF"))?;
        let bytes = fs::read(&pending)?;
        let stem = timestamped_stem(&self.file_prefix, chrono::Local::now());
        let final_path = write_exclusive_gif(&self.save_dir, &stem, &bytes)?;
        let _ = fs::remove_file(&pending);
        let path_str = final_path.to_string_lossy().to_string();
        let mut st = self.status.write();
        st.pending_path = None;
        st.saved_path = Some(path_str);
        st.file_size_bytes = bytes.len() as u64;
        Ok(final_path)
    }

    /// 读取尚未确认的临时 GIF，供预览和复制。
    pub fn read_pending(&self) -> Result<Vec<u8>, anyhow::Error> {
        let pending = self
            .status
            .read()
            .pending_path
            .clone()
            .or_else(|| self.status.read().saved_path.clone())
            .ok_or_else(|| anyhow::anyhow!("没有可读取的 GIF"))?;
        Ok(fs::read(pending)?)
    }

    /// 等待后台录制线程退出（在停用或关闭时调用）。
    pub fn wait_finished(&self, timeout: Duration) {
        self.stop_signal.store(true, Ordering::SeqCst);
        let mut guard = self.worker_handle.lock();
        if let Some(handle) = guard.take() {
            let start = Instant::now();
            while !handle.is_finished() {
                if start.elapsed() >= timeout {
                    warn!("GIF recorder thread did not exit within timeout");
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_recording_loop(
        session_id: String,
        capture_bounds: SelectionBox,
        target_w: u32,
        target_h: u32,
        config: GifRecordingConfig,
        temp_file_path: PathBuf,
        capturer: Arc<dyn ScreenCapturer>,
        overlay_png: Option<Vec<u8>>,
        exclude_window_id: u32,
        status: Arc<RwLock<GifRecordingStatus>>,
        stop_signal: Arc<AtomicBool>,
        cancel_signal: Arc<AtomicBool>,
        event_sink: Option<GifEventSink>,
    ) {
        info!(session_id = %session_id, target_w, target_h, fps = ?config.framerate, "GIF recording worker started");

        if let Err(e) = fs::create_dir_all(temp_file_path.parent().unwrap_or(std::path::Path::new("."))) {
            let msg = format!("创建 GIF 保存目录失败：{}", e);
            warn!(%msg);
            let mut st = status.write();
            st.is_recording = false;
            st.error = Some(msg);
            return;
        }

        let temp_file = match File::create(&temp_file_path) {
            Ok(f) => f,
            Err(e) => {
                let msg = format!("创建 GIF 临时文件失败：{}", e);
                warn!(%msg);
                let mut st = status.write();
                st.is_recording = false;
                st.error = Some(msg);
                return;
            }
        };

        let mut encoder = match GifStreamEncoder::new(
            temp_file,
            target_w,
            target_h,
            config.framerate,
            config.sample_factor,
        ) {
            Ok(enc) => enc,
            Err(e) => {
                let msg = format!("初始化 GIF 编码器失败：{}", e);
                warn!(%msg);
                let mut st = status.write();
                st.is_recording = false;
                st.error = Some(msg);
                let _ = fs::remove_file(&temp_file_path);
                return;
            }
        };

        let frame_interval = config.framerate.frame_interval();
        let start_time = Instant::now();
        let max_duration = Duration::from_secs(config.max_duration_secs);
        let overlay = decode_overlay(overlay_png.as_deref());
        let mut auto_stopped = false;

        while !stop_signal.load(Ordering::SeqCst) {
            let tick_start = Instant::now();
            let elapsed = start_time.elapsed();

            // 超出最长录制时长后停止，交给预览而不是直接落成最终文件。
            if elapsed >= max_duration {
                info!(session_id = %session_id, "GIF recording reached max duration limit");
                auto_stopped = true;
                break;
            }

            match capturer.capture_region_below(capture_bounds, exclude_window_id) {
                Ok(frame) => {
                    let mut pixels = frame.rgba_bytes;
                    // 排除了遮罩时，抓到的是底下的动态画面，墨迹要再叠一次。
                    // 没拿到窗口号时，抓屏已经包含遮罩上的墨迹，不能再叠。
                    if exclude_window_id != 0 {
                        if let Some(ink) = &overlay {
                            blend_overlay(&mut pixels, frame.width, frame.height, ink);
                        }
                    }
                    if let Err(e) = encoder.add_frame(&pixels, frame.width, frame.height) {
                        warn!(%e, "Failed to encode GIF frame");
                    }
                }
                Err(e) => {
                    warn!(%e, "Failed to capture region frame for GIF");
                }
            }

            let frames = encoder.encoded_frames();
            if frames >= config.max_frames {
                info!(session_id = %session_id, "GIF recording reached max frame limit");
                auto_stopped = true;
                break;
            }

            let file_size = fs::metadata(&temp_file_path).map(|m| m.len()).unwrap_or(0);

            // 更新状态与推送事件
            let dur_ms = elapsed.as_millis() as u64;
            {
                let mut st = status.write();
                st.duration_ms = dur_ms;
                st.frame_count = frames;
                st.file_size_bytes = file_size;
            }

            if let Some(sink) = &event_sink {
                let payload = serde_json::to_value(GifRecordingStatsPayload {
                    duration_ms: dur_ms,
                    frame_count: frames,
                    file_size_bytes: file_size,
                })
                .unwrap_or_default();
                sink("screenshot://gif-recording-stats", payload);
            }

            let tick_elapsed = tick_start.elapsed();
            if tick_elapsed < frame_interval {
                thread::sleep(frame_interval - tick_elapsed);
            }
        }

        // 完成编码并落盘
        let encoded_count = encoder.finish().unwrap_or(0);

        if cancel_signal.load(Ordering::SeqCst) {
            info!(session_id = %session_id, "GIF recording cancelled, deleting partial file");
            let _ = fs::remove_file(&temp_file_path);
            let mut st = status.write();
            st.is_recording = false;
        } else if encoded_count == 0 {
            info!(session_id = %session_id, "No frames captured, removing empty GIF file");
            let _ = fs::remove_file(&temp_file_path);
            let mut st = status.write();
            st.is_recording = false;
            st.auto_stopped = auto_stopped;
            st.error = Some("未录制到任何有效帧".to_string());
        } else if temp_file_path.is_file() {
            info!(session_id = %session_id, path = %temp_file_path.display(), "GIF ready for preview");
            let size = fs::metadata(&temp_file_path).map(|m| m.len()).unwrap_or(0);
            let mut st = status.write();
            st.is_recording = false;
            st.auto_stopped = auto_stopped;
            st.pending_path = Some(temp_file_path.to_string_lossy().to_string());
            st.file_size_bytes = size;
        } else {
            let mut st = status.write();
            st.is_recording = false;
            st.error = Some("GIF 临时文件没有生成".to_string());
        }
    }
}

/// 把选区大小的标注 PNG 叠到抓到的画面上。遮罩窗口已被排除，墨迹只合成这一次。
fn decode_overlay(bytes: Option<&[u8]>) -> Option<image::RgbaImage> {
    let bytes = bytes?;
    if bytes.is_empty() {
        return None;
    }
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()
        .map(|img| img.to_rgba8())
}

fn blend_overlay(frame: &mut [u8], width: u32, height: u32, overlay: &image::RgbaImage) {
    let ink = if overlay.width() == width && overlay.height() == height {
        None
    } else {
        Some(image::imageops::resize(
            overlay,
            width.max(1),
            height.max(1),
            image::imageops::FilterType::Triangle,
        ))
    };
    let ink = ink.as_ref().unwrap_or(overlay);
    for y in 0..height {
        for x in 0..width {
            let pixel = ink.get_pixel(x, y);
            let alpha = pixel[3] as u32;
            if alpha == 0 {
                continue;
            }
            let i = ((y * width + x) * 4) as usize;
            if i + 4 > frame.len() {
                return;
            }
            if alpha == 255 {
                frame[i..i + 4].copy_from_slice(&pixel.0);
                continue;
            }
            for channel in 0..3 {
                let src = pixel[channel] as u32;
                let dst = frame[i + channel] as u32;
                frame[i + channel] = ((src * alpha + dst * (255 - alpha)) / 255) as u8;
            }
            frame[i + 3] = 255;
        }
    }
}

/// 以 `.gif` 后缀调用独占创建保存。
fn write_exclusive_gif(
    dir: &std::path::Path,
    stem: &str,
    bytes: &[u8],
) -> Result<PathBuf, anyhow::Error> {
    fs::create_dir_all(dir)?;
    let mut candidate = dir.join(format!("{}.gif", stem));
    let mut counter = 1;

    while candidate.exists() {
        candidate = dir.join(format!("{}_{}.gif", stem, counter));
        counter += 1;
    }

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&candidate)?;
    file.write_all(bytes)?;
    Ok(candidate)
}
