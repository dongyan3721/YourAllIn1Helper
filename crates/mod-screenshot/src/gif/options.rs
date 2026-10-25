//! GIF 录制分辨率、帧率与参数计算。

use serde::{Deserialize, Serialize};

/// GIF 录制分辨率预设。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "preset", rename_all = "snake_case")]
pub enum GifResolutionPreset {
    /// 原画（100% 原始选区物理像素尺寸）。
    Original,
    /// 1080p（长边限制 1920，等比缩放）。
    P1080,
    /// 720p（长边限制 1280，等比缩放）。
    P720,
    /// 480p（长边限制 854，等比缩放）。
    P480,
    /// 自定义宽高（保持或限制选区比例）。
    Custom { width: u32, height: u32 },
}

impl Default for GifResolutionPreset {
    fn default() -> Self {
        Self::Original
    }
}

/// 支持的 GIF 录制帧率。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GifFramerate {
    Fps10 = 10,
    Fps15 = 15,
    Fps24 = 24,
    Fps30 = 30,
}

impl Default for GifFramerate {
    fn default() -> Self {
        Self::Fps15
    }
}

impl GifFramerate {
    pub fn fps_value(&self) -> u32 {
        *self as u32
    }

    /// GIF 规范中的帧延迟（单位：1/100 秒，即 10ms）。
    pub fn delay_centisecs(&self) -> u16 {
        match self {
            Self::Fps10 => 10, // 100ms
            Self::Fps15 => 7,  // ~67ms
            Self::Fps24 => 4,  // ~42ms
            Self::Fps30 => 3,  // ~33ms
        }
    }

    /// 帧间隔时长。
    pub fn frame_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(1000 / self.fps_value() as u64)
    }

    pub fn from_u32(val: u32) -> Self {
        match val {
            0..=12 => Self::Fps10,
            13..=19 => Self::Fps15,
            20..=27 => Self::Fps24,
            _ => Self::Fps30,
        }
    }
}

/// 根据预设与原始选区尺寸计算目标输出分辨率（严格保持宽高比，且保证宽高为偶数与至少为 2）。
pub fn calculate_target_dimensions(
    orig_w: u32,
    orig_h: u32,
    preset: &GifResolutionPreset,
) -> (u32, u32) {
    if orig_w == 0 || orig_h == 0 {
        return (2, 2);
    }

    let (tw, th) = match preset {
        GifResolutionPreset::Original => (orig_w, orig_h),
        GifResolutionPreset::P1080 => scale_down_max_dim(orig_w, orig_h, 1920),
        GifResolutionPreset::P720 => scale_down_max_dim(orig_w, orig_h, 1280),
        GifResolutionPreset::P480 => scale_down_max_dim(orig_w, orig_h, 854),
        GifResolutionPreset::Custom { width, height } => {
            let w = (*width).max(2);
            let h = (*height).max(2);
            (w, h)
        }
    };

    // 规范化：确保为偶数且至少为 2
    let even_w = ((tw.max(2) + 1) / 2) * 2;
    let even_h = ((th.max(2) + 1) / 2) * 2;
    (even_w, even_h)
}

fn scale_down_max_dim(orig_w: u32, orig_h: u32, max_dim: u32) -> (u32, u32) {
    let max_orig = orig_w.max(orig_h);
    if max_orig <= max_dim {
        return (orig_w, orig_h);
    }
    let ratio = (max_dim as f64) / (max_orig as f64);
    let tw = ((orig_w as f64) * ratio).round() as u32;
    let th = ((orig_h as f64) * ratio).round() as u32;
    (tw.max(2), th.max(2))
}

/// GIF 录制配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GifRecordingConfig {
    pub resolution_preset: GifResolutionPreset,
    pub framerate: GifFramerate,
    /// 最长录制时间（秒），默认 120 秒。
    pub max_duration_secs: u64,
    /// 最大允许帧数，默认 3000。
    pub max_frames: usize,
    /// NeuQuant 量化采样因子（1~30，数值越小画质越好但计算稍重，推荐 10）。
    pub sample_factor: i32,
}

impl Default for GifRecordingConfig {
    fn default() -> Self {
        Self {
            resolution_preset: GifResolutionPreset::Original,
            framerate: GifFramerate::Fps15,
            max_duration_secs: 120,
            max_frames: 3000,
            sample_factor: 10,
        }
    }
}
