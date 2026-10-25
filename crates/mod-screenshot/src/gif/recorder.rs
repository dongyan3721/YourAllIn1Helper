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
    pub saved_path: Option<String>,
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
    status: Arc<RwLock<GifRecordingStatus>>,
    stop_signal: Arc<AtomicBool>,
    cancel_signal: Arc<AtomicBool>,
    worker_handle: Mutex<Option<JoinHandle<()>>>,
}

impl GifRecordingSession {
    pub fn new(
        session_id: String,
        selection: SelectionBox,
        save_dir: PathBuf,
        file_prefix: String,
        config: GifRecordingConfig,
        capturer: Arc<dyn ScreenCapturer>,
        event_sink: Option<GifEventSink>,
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
            saved_path: None,
            error: None,
        }));

        let session = Arc::new(Self {
            session_id: session_id.clone(),
            selection,
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
                    selection,
                    target_w,
                    target_h,
                    config,
                    save_dir,
                    file_prefix,
                    temp_file_path,
                    capturer,
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
        selection: SelectionBox,
        target_w: u32,
        target_h: u32,
        config: GifRecordingConfig,
        save_dir: PathBuf,
        file_prefix: String,
        temp_file_path: PathBuf,
        capturer: Arc<dyn ScreenCapturer>,
        status: Arc<RwLock<GifRecordingStatus>>,
        stop_signal: Arc<AtomicBool>,
        cancel_signal: Arc<AtomicBool>,
        event_sink: Option<GifEventSink>,
    ) {
        info!(session_id = %session_id, target_w, target_h, fps = ?config.framerate, "GIF recording worker started");

        if let Err(e) = fs::create_dir_all(&save_dir) {
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

        while !stop_signal.load(Ordering::SeqCst) {
            let tick_start = Instant::now();
            let elapsed = start_time.elapsed();

            // 超出最长录制时长保护
            if elapsed >= max_duration {
                info!(session_id = %session_id, "GIF recording reached max duration limit");
                break;
            }

            // 1. 抓取选区
            match capturer.capture_region(selection) {
                Ok(frame) => {
                    if let Err(e) = encoder.add_frame(&frame.rgba_bytes, frame.width, frame.height)
                    {
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
            st.error = Some("未录制到任何有效帧".to_string());
        } else {
            // 读取半成品并按 create_new 独占写入最终文件
            let read_bytes = fs::read(&temp_file_path);
            let _ = fs::remove_file(&temp_file_path);

            match read_bytes {
                Ok(bytes) => {
                    let stem = timestamped_stem(&file_prefix, chrono::Local::now());
                    match write_exclusive_gif(&save_dir, &stem, &bytes) {
                        Ok(final_path) => {
                            let path_str = final_path.to_string_lossy().to_string();
                            info!(session_id = %session_id, path = %path_str, "GIF saved successfully");
                            let mut st = status.write();
                            st.is_recording = false;
                            st.saved_path = Some(path_str);
                            st.file_size_bytes = bytes.len() as u64;
                        }
                        Err(e) => {
                            let msg = format!("保存 GIF 文件失败：{}", e);
                            warn!(%msg);
                            let mut st = status.write();
                            st.is_recording = false;
                            st.error = Some(msg);
                        }
                    }
                }
                Err(e) => {
                    let msg = format!("读取临时 GIF 数据失败：{}", e);
                    warn!(%msg);
                    let mut st = status.write();
                    st.is_recording = false;
                    st.error = Some(msg);
                }
            }
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
