//! 滚动截图拼接状态机。
//!
//! 维护拼接画布缓冲、位移累加、处理重复帧、固定页眉页脚、反向滚动检测、
//! 低置信度安全熔断、长度与内存上限约束。

use super::ncc::{
    calculate_displacement, detect_fixed_footer, detect_fixed_header, DisplacementMatch, NccConfig,
};
use serde::{Deserialize, Serialize};

/// 拼接停止原因。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum StopReason {
    /// 用户主动停止。
    UserStopped,
    /// 用户主动取消。
    Cancelled,
    /// 达到最大长度上限（如 30000 像素）。
    MaxLengthReached { max_height: usize },
    /// 达到内存上限。
    MaxMemoryReached { max_bytes: usize },
    /// 匹配置信度过低，安全熔断以保留已有成果，不产出错位长图。
    LowConfidence {
        confidence: f64,
        description: String,
    },
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserStopped => write!(f, "用户主动完成长截图"),
            Self::Cancelled => write!(f, "用户取消长截图"),
            Self::MaxLengthReached { max_height } => {
                write!(
                    f,
                    "达到最大长度上限（{} 像素），已自动保存已拼部分",
                    max_height
                )
            }
            Self::MaxMemoryReached { max_bytes } => {
                write!(
                    f,
                    "达到内存占用上限（{} MB），已自动保存已拼部分",
                    max_bytes / 1024 / 1024
                )
            }
            Self::LowConfidence {
                confidence,
                description,
            } => {
                write!(
                    f,
                    "画面匹配置信度过低（{:.2}，{}），为避免错位已停止拼接并保留已有部分",
                    confidence, description
                )
            }
        }
    }
}

/// 单次加帧的拼接结果。
#[derive(Debug, Clone, PartialEq)]
pub enum StitchStepResult {
    /// 成功拼接新增 $\Delta y$ 像素。
    Stitched {
        dy: usize,
        current_height: usize,
        confidence: f64,
    },
    /// 无位移或重复帧，未产生新内容。
    ZeroDisplacement,
    /// 检测到反向滚动（向上倒滚），跳过。
    ReverseScroll,
    /// 拼接因达到限制或低置信度已安全停止。
    Stopped(StopReason),
}

/// 拼接配置约束。
#[derive(Debug, Clone)]
pub struct StitcherConfig {
    /// 最大允许高度（像素），默认 30,000。
    pub max_height: usize,
    /// 最大允许内存（字节），默认 512 MB (512 * 1024 * 1024)。
    pub max_memory_bytes: usize,
    /// 允许的最大连续低置信度跳过次数，默认 2 次。
    pub max_low_confidence_skips: usize,
    /// NCC 匹配参数。
    pub ncc: NccConfig,
}

impl Default for StitcherConfig {
    fn default() -> Self {
        Self {
            max_height: 30_000,
            max_memory_bytes: 512 * 1024 * 1024,
            max_low_confidence_skips: 2,
            ncc: NccConfig::default(),
        }
    }
}

/// 最终生成的长截图数据。
#[derive(Debug, Clone)]
pub struct StitchedImage {
    pub width: u32,
    pub height: u32,
    pub rgba_bytes: Vec<u8>,
    pub frame_count: usize,
    pub stop_reason: StopReason,
}

/// 滚动拼接器。
pub struct ScrollStitcher {
    config: StitcherConfig,
    width: usize,
    initial_frame_height: usize,
    /// 已拼接的主画布 RGBA 字节流。
    stitched_rgba: Vec<u8>,
    /// 当前已拼接高度。
    current_height: usize,
    /// 前一帧完整原始像素。
    prev_frame: Option<Vec<u8>>,
    /// 最后一帧完整原始像素（用于结束时提取固定页脚）。
    last_frame: Option<Vec<u8>>,
    /// 固定的顶部导航栏高度。
    fixed_header: usize,
    /// 固定的底部栏高度。
    fixed_footer: usize,
    /// 是否已完成头尾静止区域判定。
    header_footer_initialized: bool,
    /// 已处理帧数计数。
    frame_count: usize,
    /// 连续低置信度帧计数。
    low_confidence_skips: usize,
    /// 拼接是否已结束。
    finished: bool,
    /// 结束原因。
    stop_reason: Option<StopReason>,
}

impl ScrollStitcher {
    pub fn new(config: StitcherConfig) -> Self {
        Self {
            config,
            width: 0,
            initial_frame_height: 0,
            stitched_rgba: Vec::new(),
            current_height: 0,
            prev_frame: None,
            last_frame: None,
            fixed_header: 0,
            fixed_footer: 0,
            header_footer_initialized: false,
            frame_count: 0,
            low_confidence_skips: 0,
            finished: false,
            stop_reason: None,
        }
    }

    /// 当前画布高度。
    pub fn current_height(&self) -> usize {
        self.current_height
    }

    /// 帧宽度。
    pub fn width(&self) -> usize {
        self.width
    }

    /// 已累计处理的帧数。
    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    /// 是否已经停止或结束。
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// 接收新的一帧进行比对和拼接。
    pub fn push_frame(&mut self, frame: &[u8], width: usize, height: usize) -> StitchStepResult {
        if self.finished {
            return StitchStepResult::Stopped(
                self.stop_reason.clone().unwrap_or(StopReason::UserStopped),
            );
        }

        // 首帧初始化
        if self.prev_frame.is_none() {
            self.width = width;
            self.initial_frame_height = height;
            self.stitched_rgba = frame.to_vec();
            self.current_height = height;
            self.prev_frame = Some(frame.to_vec());
            self.last_frame = Some(frame.to_vec());
            self.frame_count = 1;

            return StitchStepResult::Stitched {
                dy: 0,
                current_height: height,
                confidence: 1.0,
            };
        }

        let prev = self.prev_frame.as_ref().unwrap();

        // 优先检查完全重复帧（无位移）：不在此类帧上探测固定头尾
        if prev == frame {
            self.last_frame = Some(frame.to_vec());
            return StitchStepResult::ZeroDisplacement;
        }

        // 第二帧且产生位移时探测固定页眉与页脚
        if !self.header_footer_initialized {
            self.fixed_header = detect_fixed_header(prev, frame, width, height, &self.config.ncc);
            self.fixed_footer = detect_fixed_footer(prev, frame, width, height, &self.config.ncc);
            self.header_footer_initialized = true;

            // 若存在固定页脚，首帧末尾暂存的页脚需在动态拼接期间剥离，待最终完成后单次挂载
            if self.fixed_footer > 0 && self.current_height >= self.fixed_footer {
                let keep_height = self.current_height - self.fixed_footer;
                self.stitched_rgba.truncate(keep_height * width * 4);
                self.current_height = keep_height;
            }
        }

        // 计算相邻帧位移与置信度
        let match_result = calculate_displacement(
            prev,
            frame,
            width,
            height,
            self.fixed_header,
            self.fixed_footer,
            &self.config.ncc,
        );

        match match_result {
            Some(DisplacementMatch { dy: 0, .. }) => {
                // 重复帧/无位移
                self.last_frame = Some(frame.to_vec());
                StitchStepResult::ZeroDisplacement
            }
            Some(DisplacementMatch { dy, confidence }) if dy > 0 => {
                // 成功检测到向下位移
                self.low_confidence_skips = 0;

                // 检查高度与内存上限约束
                let new_height = self.current_height + dy;
                if new_height > self.config.max_height {
                    let reason = StopReason::MaxLengthReached {
                        max_height: self.config.max_height,
                    };
                    self.stop(reason.clone());
                    return StitchStepResult::Stopped(reason);
                }

                let new_bytes_len = new_height * width * 4;
                if new_bytes_len > self.config.max_memory_bytes {
                    let reason = StopReason::MaxMemoryReached {
                        max_bytes: self.config.max_memory_bytes,
                    };
                    self.stop(reason.clone());
                    return StitchStepResult::Stopped(reason);
                }

                // 从当前帧提取新增的 dy 行并追加到画布
                // 新出现的行位于当前帧中排除固定页脚后的底部 dy 行
                let copy_start_y = height - self.fixed_footer - dy;
                let copy_end_y = height - self.fixed_footer;

                let row_bytes = width * 4;
                let start_offset = copy_start_y * row_bytes;
                let end_offset = copy_end_y * row_bytes;

                if end_offset <= frame.len() && start_offset < end_offset {
                    self.stitched_rgba
                        .extend_from_slice(&frame[start_offset..end_offset]);
                    self.current_height = new_height;
                }

                self.prev_frame = Some(frame.to_vec());
                self.last_frame = Some(frame.to_vec());
                self.frame_count += 1;

                StitchStepResult::Stitched {
                    dy,
                    current_height: self.current_height,
                    confidence,
                }
            }
            _ => {
                // 未匹配成功或置信度过低
                self.low_confidence_skips += 1;
                if self.low_confidence_skips > self.config.max_low_confidence_skips {
                    let reason = StopReason::LowConfidence {
                        confidence: 0.0,
                        description: "无法可靠识别连续位移，停止拼接以防止图像错位".to_string(),
                    };
                    self.stop(reason.clone());
                    StitchStepResult::Stopped(reason)
                } else {
                    // 暂时跳过单帧扰动，等待下一帧比对
                    StitchStepResult::ZeroDisplacement
                }
            }
        }
    }

    /// 标记停止拼接并记录原因。
    pub fn stop(&mut self, reason: StopReason) {
        if !self.finished {
            self.finished = true;
            self.stop_reason = Some(reason);
        }
    }

    /// 结束拼接流程并产出最终图像。
    ///
    /// 若探测到固定页脚，将最后一帧的固定页脚挂接到图像最底部。
    pub fn finish(mut self, reason: Option<StopReason>) -> StitchedImage {
        let final_reason = reason
            .or(self.stop_reason)
            .unwrap_or(StopReason::UserStopped);

        // 如果存在固定页脚且有最后一帧，将最后一帧的底部挂上
        if self.fixed_footer > 0 {
            if let Some(last) = &self.last_frame {
                let row_bytes = self.width * 4;
                let footer_start = (self.initial_frame_height - self.fixed_footer) * row_bytes;
                if footer_start < last.len() {
                    self.stitched_rgba.extend_from_slice(&last[footer_start..]);
                    self.current_height += self.fixed_footer;
                }
            }
        }

        StitchedImage {
            width: self.width as u32,
            height: self.current_height as u32,
            rgba_bytes: self.stitched_rgba,
            frame_count: self.frame_count,
            stop_reason: final_reason,
        }
    }
}
