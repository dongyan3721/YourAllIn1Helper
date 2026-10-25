//! 单次截图会话：持有冻结帧、被截显示器几何与窗口级智能区域。
//!
//! 会话在用户取消、导出或模块停用时整体丢弃，冻结帧随最后一个 `Arc` 释放。
use crate::geometry::MonitorGeometry;
use app_core::{
    resolve_smart_region_at_point, DetectedRegionRect, RegionHierarchyLevel, SelectionBox,
};
use os_hal::CapturedFrame;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 显示器根节点在会话区域表中的固定 ID。
pub const MONITOR_REGION_ID: u32 = 1;
/// 窗口区域 ID 起点；ID 越小越靠前（Z 序由 HAL 返回顺序决定）。
const WINDOW_REGION_BASE_ID: u32 = 1_000;
/// 小于该尺寸（帧像素）的窗口通常是阴影、指示器等，不作为候选。
const MIN_WINDOW_REGION_PX: u32 = 8;

/// 单项智能探测能力的真实可用状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CapabilityState {
    Available,
    Disabled,
    Unavailable { reason: String },
}

/// 智能选区三级能力在本次会话中的实际状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionCapabilities {
    /// L1：系统窗口边界（HAL `RegionDetector`）。
    pub window_tree: CapabilityState,
    /// L2：无障碍控件级边界，M1 未接入原生适配器。
    pub ui_elements: CapabilityState,
    /// L3：视觉边缘候选，M1 未启用。
    pub visual_edges: CapabilityState,
}

impl RegionCapabilities {
    pub fn m1(window_tree: CapabilityState) -> Self {
        Self {
            window_tree,
            ui_elements: CapabilityState::Unavailable {
                reason: "控件级（无障碍树）探测尚未接入原生适配器，计划在 M2 实现".to_string(),
            },
            visual_edges: CapabilityState::Unavailable {
                reason: "视觉边缘候选计划在 M2 与完整智能选区一起验收".to_string(),
            },
        }
    }
}

/// 冻结截图会话。
pub struct CaptureSession {
    pub id: String,
    pub monitor: MonitorGeometry,
    pub frame: CapturedFrame,
    /// 帧像素坐标的窗口区域，按 HAL 返回的前→后 Z 序排列。
    pub windows: Vec<DetectedRegionRect>,
    pub capabilities: RegionCapabilities,
    pub target_mode: Option<String>,
    pub created_at: i64,
}

impl CaptureSession {
    pub fn new(
        monitor: MonitorGeometry,
        frame: CapturedFrame,
        detected: Vec<DetectedRegionRect>,
        capabilities: RegionCapabilities,
    ) -> Self {
        let windows = Self::project_windows(&monitor, detected);
        Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            monitor,
            frame,
            windows,
            capabilities,
            target_mode: None,
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    }

    pub fn with_target_mode(mut self, mode: Option<String>) -> Self {
        self.target_mode = mode;
        self
    }

    /// 只保留顶层窗口，换算到帧像素并裁到本显示器；保持输入顺序（前→后）。
    fn project_windows(
        monitor: &MonitorGeometry,
        detected: Vec<DetectedRegionRect>,
    ) -> Vec<DetectedRegionRect> {
        detected
            .into_iter()
            .filter(|r| r.level == RegionHierarchyLevel::TopLevelWindow)
            .filter_map(|r| {
                let rect = monitor.desktop_rect_to_frame(r.x, r.y, r.width, r.height)?;
                (rect.width >= MIN_WINDOW_REGION_PX && rect.height >= MIN_WINDOW_REGION_PX)
                    .then_some((rect, r.label))
            })
            .enumerate()
            .map(|(index, (rect, label))| DetectedRegionRect {
                region_id: WINDOW_REGION_BASE_ID + index as u32,
                parent_id: Some(MONITOR_REGION_ID),
                level: RegionHierarchyLevel::TopLevelWindow,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                label,
            })
            .collect()
    }

    /// 解析光标处的智能区域；`level_steps` 为滚轮/Tab 向父级扩展的层数。
    ///
    /// 层级求解复用 `app_core::resolve_smart_region_at_point`。该算法在同层级内按面积取最小，
    /// 不感知遮挡，因此先按 Z 序找到光标处最靠前的窗口，只把它和显示器根节点交给算法，
    /// 避免被遮挡的小窗口「穿透」前景窗口被选中。
    pub fn resolve_region(&self, x: i32, y: i32, level_steps: usize) -> Option<DetectedRegionRect> {
        let root = self.monitor.monitor_region(MONITOR_REGION_ID);
        let mut candidates = Vec::with_capacity(2);
        if let Some(front) = self.windows.iter().find(|w| w.contains_point(x, y)) {
            candidates.push(front.clone());
        }
        candidates.push(root);
        resolve_smart_region_at_point(&candidates, x, y, level_steps)
    }

    /// 前端渲染所需的会话描述（不含像素）。
    pub fn describe(&self, settings: OverlaySettings) -> SessionInfo {
        let logical = self.monitor.overlay_logical_rect();
        SessionInfo {
            session_id: self.id.clone(),
            frame_width: self.frame.width,
            frame_height: self.frame.height,
            frame_byte_length: self.frame.rgba_bytes.len(),
            device_pixel_ratio: self.monitor.device_pixel_ratio,
            logical_width: logical.width,
            logical_height: logical.height,
            monitor_id: self.monitor.monitor_id,
            monitor_name: self.monitor.name.clone(),
            window_region_count: self.windows.len(),
            capabilities: self.capabilities.clone(),
            target_mode: self.target_mode.clone(),
            settings,
            created_at: self.created_at,
        }
    }

    pub fn frame_bounds(&self) -> SelectionBox {
        SelectionBox {
            x: 0,
            y: 0,
            width: self.frame.width,
            height: self.frame.height,
        }
    }
}

/// 遮罩界面使用的开关与默认值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlaySettings {
    pub manual_selection: bool,
    pub smart_regions: bool,
    pub wheel_expand: bool,
    pub magnifier: bool,
    pub default_color: String,
    pub default_stroke_width: f64,
}

/// `screenshot_get_session` 的返回值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frame_byte_length: usize,
    pub device_pixel_ratio: f64,
    pub logical_width: f64,
    pub logical_height: f64,
    pub monitor_id: u32,
    pub monitor_name: String,
    pub window_region_count: usize,
    pub capabilities: RegionCapabilities,
    #[serde(default)]
    pub target_mode: Option<String>,
    pub settings: OverlaySettings,
    pub created_at: i64,
}

/// 会话句柄：模块内部共享，导出/取消时整体丢弃。
pub type SharedSession = Arc<CaptureSession>;

#[cfg(test)]
#[path = "../tests/unit/session.rs"]
mod tests;
