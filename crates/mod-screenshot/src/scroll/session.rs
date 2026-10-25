//! 滚动截图后台采集与拼接生命周期会话。
//!
//! 在后台线程中以约 15 FPS 循环捕获指定选区画面，通过 NCC 算法实时拼接，
//! 支持注入的 `InputSimulator` 自动驱动滚轮事件，优雅处理取消、停止与安全退出。

use super::stitcher::{
    ScrollStitcher, StitchStepResult, StitchedImage, StitcherConfig, StopReason,
};
use app_core::SelectionBox;
use os_hal::{HalError, InputSimulator, ScreenCapturer};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tracing::{info, warn};

/// 滚动截图当前状态快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollStatusBrief {
    pub is_active: bool,
    pub width: u32,
    pub current_height: u32,
    pub frame_count: usize,
    pub auto_scroll_enabled: bool,
    pub auto_scroll_error: Option<String>,
    pub stop_reason: Option<StopReason>,
}

/// 滚动事件载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollFrameStitchedPayload {
    pub dy: usize,
    pub current_height: usize,
    pub confidence: f64,
    pub frame_count: usize,
}

/// 事件下发回调抽象（避免强耦合具体 EventBus）。
pub type ScrollEventSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// 滚动长截图会话句柄。
pub struct ScrollCaptureSession {
    session_id: String,
    selection: SelectionBox,
    status: Arc<RwLock<ScrollStatusBrief>>,
    stop_signal: Arc<AtomicBool>,
    cancel_signal: Arc<AtomicBool>,
    auto_scroll_flag: Arc<AtomicBool>,
    stitched_result: Arc<Mutex<Option<StitchedImage>>>,
    preview_png: Arc<Mutex<Option<Vec<u8>>>>,
    worker_handle: Mutex<Option<JoinHandle<()>>>,
}

impl ScrollCaptureSession {
    pub fn new(
        session_id: String,
        selection: SelectionBox,
        capture_bounds: SelectionBox,
        capturer: Arc<dyn ScreenCapturer>,
        input_simulator: Option<Arc<dyn InputSimulator>>,
        config: StitcherConfig,
        event_sink: Option<ScrollEventSink>,
        exclude_window_id: u32,
    ) -> Arc<Self> {
        let stop_signal = Arc::new(AtomicBool::new(false));
        let cancel_signal = Arc::new(AtomicBool::new(false));
        let auto_scroll_flag = Arc::new(AtomicBool::new(false));
        let stitched_result = Arc::new(Mutex::new(None));
        let preview_png = Arc::new(Mutex::new(None));

        let status = Arc::new(RwLock::new(ScrollStatusBrief {
            is_active: true,
            width: selection.width,
            current_height: selection.height,
            frame_count: 0,
            auto_scroll_enabled: false,
            auto_scroll_error: None,
            stop_reason: None,
        }));

        let session = Arc::new(Self {
            session_id: session_id.clone(),
            selection,
            status: status.clone(),
            stop_signal: stop_signal.clone(),
            cancel_signal: cancel_signal.clone(),
            auto_scroll_flag: auto_scroll_flag.clone(),
            stitched_result: stitched_result.clone(),
            preview_png: preview_png.clone(),
            worker_handle: Mutex::new(None),
        });

        // 启动约 15 FPS 后台采集线程（间隔 ~66ms）
        let session_clone = session.clone();
        let handle = thread::Builder::new()
            .name("aio-screenshot-scroll".to_string())
            .spawn(move || {
                Self::run_capture_loop(
                    session_id,
                    capture_bounds,
                    capturer,
                    input_simulator,
                    config,
                    exclude_window_id,
                    status,
                    stop_signal,
                    cancel_signal,
                    auto_scroll_flag,
                    stitched_result,
                    preview_png,
                    event_sink,
                );
            })
            .expect("Failed to spawn screenshot scroll thread");

        *session_clone.worker_handle.lock() = Some(handle);
        session
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn selection(&self) -> SelectionBox {
        self.selection
    }

    pub fn status(&self) -> ScrollStatusBrief {
        self.status.read().clone()
    }

    /// 开启或关闭自动滚动。
    pub fn set_auto_scroll(&self, enabled: bool) {
        self.auto_scroll_flag.store(enabled, Ordering::SeqCst);
        let mut st = self.status.write();
        st.auto_scroll_enabled = enabled;
    }

    /// 停止长截图（正常完成），返回是否成功触发。
    pub fn stop(&self) -> bool {
        if !self.stop_signal.swap(true, Ordering::SeqCst) {
            let mut st = self.status.write();
            st.is_active = false;
            st.stop_reason = Some(StopReason::UserStopped);
            true
        } else {
            false
        }
    }

    /// 取消长截图（丢弃未完成产物）。
    pub fn cancel(&self) -> bool {
        self.cancel_signal.store(true, Ordering::SeqCst);
        self.stop_signal.store(true, Ordering::SeqCst);
        let mut st = self.status.write();
        st.is_active = false;
        st.stop_reason = Some(StopReason::Cancelled);
        *self.stitched_result.lock() = None;
        true
    }

    /// 等待后台工作线程结束（安全退出或销毁）。
    pub fn wait_finished(&self, timeout: Duration) {
        self.stop_signal.store(true, Ordering::SeqCst);
        let mut guard = self.worker_handle.lock();
        if let Some(handle) = guard.take() {
            let start = Instant::now();
            while !handle.is_finished() {
                if start.elapsed() >= timeout {
                    warn!("Screenshot scroll worker did not exit within timeout");
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    }

    /// 获取最终拼接图像（若已成功生成且未取消）。
    pub fn take_stitched_image(&self) -> Option<StitchedImage> {
        self.stitched_result.lock().take()
    }

    /// 预览拼接图像数据引用。
    pub fn get_stitched_image(&self) -> Option<StitchedImage> {
        self.stitched_result.lock().clone()
    }

    /// 录制过程中的缩略图 PNG。重复帧不会改它。
    pub fn preview_png(&self) -> Option<Vec<u8>> {
        self.preview_png.lock().clone()
    }

    #[allow(clippy::too_many_arguments)]
    fn run_capture_loop(
        session_id: String,
        capture_bounds: SelectionBox,
        capturer: Arc<dyn ScreenCapturer>,
        input_simulator: Option<Arc<dyn InputSimulator>>,
        config: StitcherConfig,
        exclude_window_id: u32,
        status: Arc<RwLock<ScrollStatusBrief>>,
        stop_signal: Arc<AtomicBool>,
        cancel_signal: Arc<AtomicBool>,
        auto_scroll_flag: Arc<AtomicBool>,
        stitched_result: Arc<Mutex<Option<StitchedImage>>>,
        preview_png: Arc<Mutex<Option<Vec<u8>>>>,
        event_sink: Option<ScrollEventSink>,
    ) {
        let frame_interval = Duration::from_millis(66); // ~15 FPS
        let mut stitcher = ScrollStitcher::new(config);
        let mut last_preview = Instant::now() - Duration::from_secs(1);

        let center_x = capture_bounds.x + (capture_bounds.width as i32) / 2;
        let center_y = capture_bounds.y + (capture_bounds.height as i32) / 2;

        info!(session_id = %session_id, bounds = ?capture_bounds, "Scroll capture worker started");

        let mut auto_scroll_counter = 0;

        while !stop_signal.load(Ordering::SeqCst) {
            let loop_start = Instant::now();

            // 1. 抓取选区
            let captured = match capturer.capture_region_below(capture_bounds, exclude_window_id) {
                Ok(frame) => frame,
                Err(error) => {
                    warn!(%error, "Failed to capture region for long screenshot");
                    thread::sleep(frame_interval);
                    continue;
                }
            };

            // 2. 喂入拼接器
            let step_result = stitcher.push_frame(
                &captured.rgba_bytes,
                captured.width as usize,
                captured.height as usize,
            );

            // 3. 处理拼接状态
            match step_result {
                StitchStepResult::Stitched {
                    dy,
                    current_height,
                    confidence,
                } => {
                    let frame_count = stitcher.frame_count();
                    {
                        let mut st = status.write();
                        st.width = stitcher.width() as u32;
                        st.current_height = current_height as u32;
                        st.frame_count = frame_count;
                    }
                    if last_preview.elapsed() >= Duration::from_millis(250) {
                        if let Some(png) = encode_preview(&stitcher) {
                            *preview_png.lock() = Some(png);
                        }
                        last_preview = Instant::now();
                    }

                    if let Some(sink) = &event_sink {
                        let payload = serde_json::to_value(ScrollFrameStitchedPayload {
                            dy,
                            current_height,
                            confidence,
                            frame_count,
                        })
                        .unwrap_or_default();
                        sink("screenshot://scroll-frame-stitched", payload);
                    }
                }
                StitchStepResult::ZeroDisplacement | StitchStepResult::ReverseScroll => {
                    // 无位移或逆向滚动，静默等待
                }
                StitchStepResult::Stopped(reason) => {
                    info!(?reason, "Scroll stitching reached terminal condition");
                    let mut st = status.write();
                    st.is_active = false;
                    st.stop_reason = Some(reason);
                    break;
                }
            }

            // 4. 自动滚动（如果启用且注入了 InputSimulator）
            if auto_scroll_flag.load(Ordering::SeqCst) {
                auto_scroll_counter += 1;
                // 每隔两帧发送一次滚轮脉冲，保持滚动平稳连续
                if auto_scroll_counter % 2 == 0 {
                    if let Some(sim) = &input_simulator {
                        // 向上滚动视角 = 内容向下滚 = 滚轮 delta 负值
                        if let Err(e) = sim.scroll_wheel_at(center_x, center_y, -120) {
                            let msg = match e {
                                HalError::PermissionDenied { guidance, .. } => {
                                    format!(
                                        "缺少辅助功能权限（{}），请在系统设置中授权或使用手动滚动",
                                        guidance
                                    )
                                }
                                HalError::Unsupported(_) => {
                                    "当前系统环境不支持自动滚轮注入，请手动滚动".to_string()
                                }
                                _ => format!("自动滚动失败：{}", e),
                            };
                            warn!(%msg, "Auto scroll failed");
                            let mut st = status.write();
                            st.auto_scroll_enabled = false;
                            st.auto_scroll_error = Some(msg);
                            auto_scroll_flag.store(false, Ordering::SeqCst);
                        }
                    } else {
                        let mut st = status.write();
                        st.auto_scroll_enabled = false;
                        st.auto_scroll_error =
                            Some("未配置系统输入模拟器，只能手动滚动".to_string());
                        auto_scroll_flag.store(false, Ordering::SeqCst);
                    }
                }
            }

            // 保持帧率稳定
            let elapsed = loop_start.elapsed();
            if elapsed < frame_interval {
                thread::sleep(frame_interval - elapsed);
            }
        }

        // 会话退出结算
        if cancel_signal.load(Ordering::SeqCst) {
            info!(session_id = %session_id, "Scroll capture cancelled, discarded frames");
            let mut st = status.write();
            st.is_active = false;
            st.stop_reason = Some(StopReason::Cancelled);
            *stitched_result.lock() = None;
        } else {
            if let Some(png) = encode_preview(&stitcher) {
                *preview_png.lock() = Some(png);
            }
            let final_reason = status.read().stop_reason.clone();
            let final_image = stitcher.finish(final_reason);
            info!(
                session_id = %session_id,
                total_w = final_image.width,
                total_h = final_image.height,
                frames = final_image.frame_count,
                stop_reason = ?final_image.stop_reason,
                "Scroll capture completed successfully"
            );
            {
                let mut st = status.write();
                st.is_active = false;
                st.current_height = final_image.height;
                st.frame_count = final_image.frame_count;
                st.stop_reason = Some(final_image.stop_reason.clone());
            }
            *stitched_result.lock() = Some(final_image);
        }
    }
}

fn encode_preview(stitcher: &ScrollStitcher) -> Option<Vec<u8>> {
    let (width, height, rgba) = stitcher.downscaled_rgba(220)?;
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    image::ImageEncoder::write_image(
        encoder,
        &rgba,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .ok()?;
    Some(png)
}
