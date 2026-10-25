//! 坐标空间换算与裁剪。
//!
//! 涉及三套坐标：
//! - **桌面单位**：HAL 返回的显示器与窗口边界。macOS 为全局逻辑点（左上原点，可为负），
//!   Windows 为物理像素。
//! - **帧像素**：冻结帧 RGBA 的物理像素，以被截显示器左上角为原点。前后端交换的选区、
//!   智能区域、标注坐标统一使用该空间，裁剪不再经过任何缩放。
//! - **逻辑点**：Tauri 窗口位置/尺寸使用的坐标，用于摆放遮罩与贴图窗口。
//!
//! 不按平台分支，而是由「帧像素宽 / 桌面单位宽」与 HAL 报告的缩放因子推导换算比例，
//! 因此 Retina（帧宽 = 2 × 桌面宽）与 Windows 高 DPI（帧宽 = 桌面宽）共用同一公式。
use crate::error::internal;
use app_core::{CoreResult, DetectedRegionRect, RegionHierarchyLevel, SelectionBox};
use os_hal::DisplayMonitorInfo;
use serde::{Deserialize, Serialize};

/// 被截显示器的几何信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorGeometry {
    pub monitor_id: u32,
    pub name: String,
    pub desktop_x: i32,
    pub desktop_y: i32,
    pub desktop_width: u32,
    pub desktop_height: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    /// 设备像素比（帧像素 / 逻辑点）。
    pub device_pixel_ratio: f64,
}

/// 逻辑点矩形（Tauri 窗口坐标）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl MonitorGeometry {
    /// 由 HAL 显示器信息与实际冻结帧尺寸建立几何；尺寸为 0 时拒绝，避免后续除零。
    pub fn new(info: &DisplayMonitorInfo, frame_width: u32, frame_height: u32) -> CoreResult<Self> {
        if info.width == 0 || info.height == 0 || frame_width == 0 || frame_height == 0 {
            return Err(internal(format!(
                "显示器或冻结帧尺寸无效：桌面 {}x{}，帧 {}x{}",
                info.width, info.height, frame_width, frame_height
            )));
        }
        let ratio_from_frame = frame_width as f64 / info.width as f64;
        // HAL 缩放因子异常（0、NaN）时退回由帧宽推导，至少保证坐标比例自洽。
        let device_pixel_ratio = if info.scale_factor.is_finite() && info.scale_factor > 0.1 {
            info.scale_factor
        } else {
            ratio_from_frame.max(0.1)
        };
        Ok(Self {
            monitor_id: info.id,
            name: info.name.clone(),
            desktop_x: info.x,
            desktop_y: info.y,
            desktop_width: info.width,
            desktop_height: info.height,
            frame_width,
            frame_height,
            device_pixel_ratio,
        })
    }

    /// 帧像素 / 桌面单位（macOS Retina 为 2，Windows 为 1）。
    fn pixels_per_desktop_x(&self) -> f64 {
        self.frame_width as f64 / self.desktop_width as f64
    }

    fn pixels_per_desktop_y(&self) -> f64 {
        self.frame_height as f64 / self.desktop_height as f64
    }

    /// 桌面单位 → 逻辑点的比例（macOS 为 1，Windows 为 1 / 缩放因子）。
    fn logical_per_desktop(&self) -> f64 {
        self.pixels_per_desktop_x() / self.device_pixel_ratio
    }

    /// 遮罩窗口应覆盖的逻辑矩形（即整个被截显示器）。
    pub fn overlay_logical_rect(&self) -> LogicalRect {
        let k = self.logical_per_desktop();
        LogicalRect {
            x: self.desktop_x as f64 * k,
            y: self.desktop_y as f64 * k,
            width: self.frame_width as f64 / self.device_pixel_ratio,
            height: self.frame_height as f64 / self.device_pixel_ratio,
        }
    }

    /// 帧像素选区 → 全局逻辑点矩形，用于把贴图放在原截图位置。
    pub fn frame_to_logical(&self, selection: &SelectionBox) -> LogicalRect {
        let origin = self.overlay_logical_rect();
        LogicalRect {
            x: origin.x + selection.x as f64 / self.device_pixel_ratio,
            y: origin.y + selection.y as f64 / self.device_pixel_ratio,
            width: selection.width as f64 / self.device_pixel_ratio,
            height: selection.height as f64 / self.device_pixel_ratio,
        }
    }

    /// 桌面单位矩形（窗口边界）→ 帧像素矩形，并裁到帧范围内；完全不相交时返回 `None`。
    pub fn desktop_rect_to_frame(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Option<SelectionBox> {
        let sx = self.pixels_per_desktop_x();
        let sy = self.pixels_per_desktop_y();
        let left = ((x as f64 - self.desktop_x as f64) * sx).round() as i64;
        let top = ((y as f64 - self.desktop_y as f64) * sy).round() as i64;
        let right = left + (width as f64 * sx).round() as i64;
        let bottom = top + (height as f64 * sy).round() as i64;
        intersect_with_frame(
            left,
            top,
            right,
            bottom,
            self.frame_width,
            self.frame_height,
        )
    }

    /// 帧像素坐标 → 桌面单位坐标（供 L2 无障碍命中查询使用）。
    pub fn frame_point_to_desktop(&self, x: i32, y: i32) -> (i32, i32) {
        (
            (self.desktop_x as f64 + x as f64 / self.pixels_per_desktop_x()).round() as i32,
            (self.desktop_y as f64 + y as f64 / self.pixels_per_desktop_y()).round() as i32,
        )
    }

    /// 代表整个显示器的智能区域（层级树根节点）。
    pub fn monitor_region(&self, region_id: u32) -> DetectedRegionRect {
        DetectedRegionRect {
            region_id,
            parent_id: None,
            level: RegionHierarchyLevel::Monitor,
            x: 0,
            y: 0,
            width: self.frame_width,
            height: self.frame_height,
            label: Some(self.name.clone()),
        }
    }
}

/// 把任意边界（可能为负或超出帧）裁剪到帧范围，空交集返回 `None`。
fn intersect_with_frame(
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
    frame_width: u32,
    frame_height: u32,
) -> Option<SelectionBox> {
    let l = left.clamp(0, frame_width as i64);
    let t = top.clamp(0, frame_height as i64);
    let r = right.clamp(0, frame_width as i64);
    let b = bottom.clamp(0, frame_height as i64);
    if r <= l || b <= t {
        return None;
    }
    Some(SelectionBox {
        x: l as i32,
        y: t as i32,
        width: (r - l) as u32,
        height: (b - t) as u32,
    })
}

/// 校验并规范化前端提交的帧像素选区：裁到帧内，交集为空或不足 1px 时拒绝。
pub fn normalize_crop(
    selection: &SelectionBox,
    frame_width: u32,
    frame_height: u32,
) -> CoreResult<SelectionBox> {
    let left = selection.x as i64;
    let top = selection.y as i64;
    intersect_with_frame(
        left,
        top,
        left + selection.width as i64,
        top + selection.height as i64,
        frame_width,
        frame_height,
    )
    .ok_or_else(|| internal("选区为空或完全位于截图范围之外"))
}

/// 从 RGBA8 冻结帧中按行复制选区像素（选区须已经过 `normalize_crop`）。
pub fn crop_rgba(
    rgba: &[u8],
    frame_width: u32,
    frame_height: u32,
    selection: &SelectionBox,
) -> CoreResult<Vec<u8>> {
    let expected = frame_width as usize * frame_height as usize * 4;
    if rgba.len() != expected {
        return Err(internal(format!(
            "冻结帧像素长度不匹配：期望 {} 字节，实际 {} 字节",
            expected,
            rgba.len()
        )));
    }
    let inside = selection.x >= 0
        && selection.y >= 0
        && selection.x as u64 + selection.width as u64 <= frame_width as u64
        && selection.y as u64 + selection.height as u64 <= frame_height as u64;
    if !inside || selection.width == 0 || selection.height == 0 {
        return Err(internal("裁剪区域超出冻结帧边界"));
    }
    let row_bytes = selection.width as usize * 4;
    let stride = frame_width as usize * 4;
    let mut out = Vec::with_capacity(row_bytes * selection.height as usize);
    for row in 0..selection.height as usize {
        let start = (selection.y as usize + row) * stride + selection.x as usize * 4;
        out.extend_from_slice(&rgba[start..start + row_bytes]);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "../tests/unit/geometry.rs"]
mod tests;
