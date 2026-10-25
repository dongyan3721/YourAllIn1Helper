//! 单次截图会话：持有冻结帧、被截显示器几何与窗口级智能区域。
//!
//! 会话在用户取消、导出或模块停用时整体丢弃，冻结帧随最后一个 `Arc` 释放。
use crate::geometry::MonitorGeometry;
use app_core::{
    infer_visual_edge_box_rgba, resolve_smart_region_at_point, DetectedRegionRect,
    RegionHierarchyLevel, SelectionBox,
};

/// 两个帧像素矩形的交集。
fn intersect(a: &SelectionBox, b: &SelectionBox) -> Option<SelectionBox> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x as i64 + a.width as i64).min(b.x as i64 + b.width as i64);
    let bottom = (a.y as i64 + a.height as i64).min(b.y as i64 + b.height as i64);
    (right > left as i64 && bottom > top as i64).then(|| SelectionBox {
        x: left,
        y: top,
        width: (right - left as i64) as u32,
        height: (bottom - top as i64) as u32,
    })
}
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
    pub fn new(
        window_tree: CapabilityState,
        ui_elements: CapabilityState,
        visual_edges: CapabilityState,
    ) -> Self {
        Self {
            window_tree,
            ui_elements,
            visual_edges,
        }
    }

    /// 智能选区关闭时三级能力全部为 Disabled。
    pub fn disabled() -> Self {
        Self::new(
            CapabilityState::Disabled,
            CapabilityState::Disabled,
            CapabilityState::Disabled,
        )
    }

    pub fn ui_available(&self) -> bool {
        self.ui_elements == CapabilityState::Available
    }

    pub fn visual_available(&self) -> bool {
        self.visual_edges == CapabilityState::Available
    }
}

/// L2 控件区域 ID 起点（会话内编号）。
const UI_REGION_BASE_ID: u32 = 2_000;
/// L3 视觉候选区域 ID。
const VISUAL_REGION_ID: u32 = 3_000;
/// L3 亮度对比阈值：越大越保守，只吸附明显的卡片/面板边界。
const VISUAL_EDGE_CONTRAST: u8 = 28;
/// L3 候选最小边长（帧像素），过滤文字笔画等细碎边缘。
const VISUAL_MIN_SIDE_PX: u32 = 24;

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
        self.resolve_region_with_ui(x, y, level_steps, &[])
    }

    /// 光标处最前方窗口（帧像素）。
    pub fn front_window_at(&self, x: i32, y: i32) -> Option<&DetectedRegionRect> {
        self.windows.iter().find(|w| w.contains_point(x, y))
    }

    /// 带 L2 控件链的层级求解。`ui_chain` 为 HAL 返回的桌面单位矩形（叶子在前）。
    ///
    /// 候选树：叶子控件 → 父级容器… → 最前窗口 → 显示器；每级都裁到所属窗口内。
    /// L2 无结果且视觉候选可用时，用 `app_core::infer_visual_edge_box_rgba` 在冻结帧上推导一个
    /// 窗口内的视觉边框作为叶子（L3）。最终仍交给 `resolve_smart_region_at_point` 选择与上卷。
    pub fn resolve_region_with_ui(
        &self,
        x: i32,
        y: i32,
        level_steps: usize,
        ui_chain: &[DetectedRegionRect],
    ) -> Option<DetectedRegionRect> {
        let root = self.monitor.monitor_region(MONITOR_REGION_ID);
        let front = self.front_window_at(x, y).cloned();
        let container = front
            .as_ref()
            .map(|w| w.to_selection_box())
            .unwrap_or_else(|| self.frame_bounds());
        let top_parent = front
            .as_ref()
            .map(|w| w.region_id)
            .unwrap_or(MONITOR_REGION_ID);

        // L2：投影到帧像素并裁到窗口内，按叶子→容器建立 parent 链。
        let projected: Vec<DetectedRegionRect> = ui_chain
            .iter()
            .filter_map(|r| {
                let rect = self
                    .monitor
                    .desktop_rect_to_frame(r.x, r.y, r.width, r.height)?;
                let rect = intersect(&rect, &container)?;
                (rect.width >= 2 && rect.height >= 2).then(|| DetectedRegionRect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    ..r.clone()
                })
            })
            .collect();
        let mut candidates: Vec<DetectedRegionRect> = Vec::with_capacity(projected.len() + 3);
        let count = projected.len();
        for (index, mut region) in projected.into_iter().enumerate() {
            region.region_id = UI_REGION_BASE_ID + index as u32;
            region.parent_id = Some(if index + 1 < count {
                UI_REGION_BASE_ID + index as u32 + 1
            } else {
                top_parent
            });
            region.level = if index == 0 {
                RegionHierarchyLevel::UiElement
            } else {
                RegionHierarchyLevel::ContainerPanel
            };
            candidates.push(region);
        }

        // L3：没有控件信息时的视觉边缘兜底，严格限制尺寸，避免吸附到文字笔画或整窗。
        if candidates.is_empty() && self.capabilities.visual_available() {
            if let Some(visual) = self.visual_candidate(x, y, &container, top_parent) {
                candidates.push(visual);
            }
        }
        if let Some(window) = front {
            candidates.push(window);
        }
        candidates.push(root);
        resolve_smart_region_at_point(&candidates, x, y, level_steps)
    }

    fn visual_candidate(
        &self,
        x: i32,
        y: i32,
        container: &SelectionBox,
        parent_id: u32,
    ) -> Option<DetectedRegionRect> {
        if x < 0 || y < 0 {
            return None;
        }
        let found = infer_visual_edge_box_rgba(
            &self.frame.rgba_bytes,
            self.frame.width,
            self.frame.height,
            x as u32,
            y as u32,
            VISUAL_EDGE_CONTRAST,
        )?;
        let rect = intersect(&found.to_selection_box(), container)?;
        let container_area = container.width as u64 * container.height as u64;
        let area = rect.width as u64 * rect.height as u64;
        let acceptable = rect.width >= VISUAL_MIN_SIDE_PX
            && rect.height >= VISUAL_MIN_SIDE_PX
            && area * 10 < container_area * 9;
        acceptable.then(|| DetectedRegionRect {
            region_id: VISUAL_REGION_ID,
            parent_id: Some(parent_id),
            level: RegionHierarchyLevel::UiElement,
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            label: Some("visual_edge".to_string()),
        })
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
            intent: crate::ocr::CaptureIntent::Capture,
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
    /// 触发意图：`ocr` 时遮罩按 Enter 识别文字而不是复制图片。
    pub intent: crate::ocr::CaptureIntent,
}

/// 会话句柄：模块内部共享，导出/取消时整体丢弃。
pub type SharedSession = Arc<CaptureSession>;

#[cfg(test)]
#[path = "../tests/unit/session.rs"]
mod tests;
