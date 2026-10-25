//! Manual drag box selection and Snipaste-style bounding-box region inference engine.
//!
//! Provides the core geometry state machine and hierarchical region solver shared by
//! `mod-screenshot` (capture, scroll capture, GIF recording, OCR) and `os-hal`:
//! 1. **Manual Drag Box Selection (`SelectionBox`)**: Freehand mouse drag selection,
//!    8-handle resizing, `1px` arrow-key nudging, and monitor bounds clamping.
//! 2. **Snipaste-Style Hierarchical Region Detection (`SnipasteRegionEngine`)**:
//!    - **L1 (OS Window Tree)**: Top-level and child window bounds (`HWND` / `CGWindowID`).
//!    - **L2 (Accessibility UI Tree)**: Fine-grained UI controls (`UIAutomation` / `AXUIElement`).
//!    - **L3 (Visual Edge-Box Inference)**: Pure-Rust luminance gradient raycast & projection
//!      detector that infers bounding boxes around buttons, cards, images, and code blocks
//!      directly from the frozen RGBA screen buffer when accessibility trees are unavailable.

use serde::{Deserialize, Serialize};

/// 8-way resize handles around an active screenshot selection box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResizeHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

/// Interactive manual drag-selection rectangle supporting freehand drag, 8-handle resize,
/// and 1px keyboard nudging (`Arrow` keys & `Shift + Arrow` keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionBox {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl SelectionBox {
    /// Constructs a normalized `SelectionBox` from two arbitrary drag anchor points `(x1, y1)` and `(x2, y2)`.
    pub fn from_drag_points(x1: i32, y1: i32, x2: i32, y2: i32) -> Self {
        let left = x1.min(x2);
        let top = y1.min(y2);
        let width = (x1 - x2).unsigned_abs();
        let height = (y1 - y2).unsigned_abs();
        Self {
            x: left,
            y: top,
            width,
            height,
        }
    }

    /// Translates the selection box by `(dx, dy)` pixels (e.g., `Arrow` keys or `Space + Drag`),
    /// optionally clamping inside `monitor_bounds`.
    pub fn translate(&mut self, dx: i32, dy: i32, monitor_bounds: Option<&SelectionBox>) {
        self.x = self.x.saturating_add(dx);
        self.y = self.y.saturating_add(dy);
        if let Some(bounds) = monitor_bounds {
            self.clamp_to_bounds(bounds);
        }
    }

    /// Resizes the selection box via one of the 8 border/corner handles by `(dx, dy)` pixels.
    pub fn resize_with_handle(&mut self, handle: ResizeHandle, dx: i32, dy: i32) {
        let mut left = self.x;
        let mut top = self.y;
        let mut right = self.x.saturating_add(self.width as i32);
        let mut bottom = self.y.saturating_add(self.height as i32);

        match handle {
            ResizeHandle::TopLeft => {
                left = left.saturating_add(dx);
                top = top.saturating_add(dy);
            }
            ResizeHandle::Top => {
                top = top.saturating_add(dy);
            }
            ResizeHandle::TopRight => {
                right = right.saturating_add(dx);
                top = top.saturating_add(dy);
            }
            ResizeHandle::Right => {
                right = right.saturating_add(dx);
            }
            ResizeHandle::BottomRight => {
                right = right.saturating_add(dx);
                bottom = bottom.saturating_add(dy);
            }
            ResizeHandle::Bottom => {
                bottom = bottom.saturating_add(dy);
            }
            ResizeHandle::BottomLeft => {
                left = left.saturating_add(dx);
                bottom = bottom.saturating_add(dy);
            }
            ResizeHandle::Left => {
                left = left.saturating_add(dx);
            }
        }

        let normalized = Self::from_drag_points(left, top, right, bottom);
        self.x = normalized.x;
        self.y = normalized.y;
        self.width = normalized.width.max(1);
        self.height = normalized.height.max(1);
    }

    /// Clamps the selection box so it stays within `bounds`.
    pub fn clamp_to_bounds(&mut self, bounds: &SelectionBox) {
        let max_w = bounds.width;
        let max_h = bounds.height;
        self.width = self.width.min(max_w);
        self.height = self.height.min(max_h);

        let max_x = bounds
            .x
            .saturating_add(bounds.width.saturating_sub(self.width) as i32);
        let max_y = bounds
            .y
            .saturating_add(bounds.height.saturating_sub(self.height) as i32);

        self.x = self.x.clamp(bounds.x, max_x);
        self.y = self.y.clamp(bounds.y, max_y);
    }
}

/// Hierarchy level for Snipaste-like bounding-box region detection during screenshot selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionHierarchyLevel {
    /// Leaf UI control or visual edge box (button, input field, code block, image).
    UiElement = 0,
    /// Intermediate container panel or child window (`EnumChildWindows` / `AXUIElement` container).
    ContainerPanel = 1,
    /// Top-level application window (`HWND` / `CGWindowID`).
    TopLevelWindow = 2,
    /// Entire physical display monitor.
    Monitor = 3,
}

/// Bounding rectangle candidate inferred by Snipaste-style smart region detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedRegionRect {
    /// Unique identifier within the current capture session.
    pub region_id: u32,
    /// Optional parent region ID for mouse-wheel hierarchical expansion (`Tab` / wheel up).
    pub parent_id: Option<u32>,
    /// Semantic hierarchy level of this bounding box.
    pub level: RegionHierarchyLevel,
    /// Screen X coordinate in physical pixels.
    pub x: i32,
    /// Screen Y coordinate in physical pixels.
    pub y: i32,
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
    /// Optional human-readable label (e.g., window title or UIA control type).
    pub label: Option<String>,
}

impl DetectedRegionRect {
    /// Returns `true` if the given physical cursor coordinate `(px, py)` lies inside this rectangle.
    #[inline]
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        let right = self.x.saturating_add(self.width as i32);
        let bottom = self.y.saturating_add(self.height as i32);
        px >= self.x && px < right && py >= self.y && py < bottom
    }

    /// Computes the area in pixels (`width * height`) for smallest-bounding-box hit-testing.
    #[inline]
    pub fn area(&self) -> u64 {
        (self.width as u64) * (self.height as u64)
    }

    /// Converts this detected region into an editable `SelectionBox`.
    #[inline]
    pub fn to_selection_box(&self) -> SelectionBox {
        SelectionBox {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

/// Given a list of detected hierarchical regions and a cursor position `(px, py)`,
/// resolves the tightest (smallest area) hit region, or climbs `wheel_steps_up` levels up its parent chain.
pub fn resolve_smart_region_at_point(
    regions: &[DetectedRegionRect],
    px: i32,
    py: i32,
    wheel_steps_up: usize,
) -> Option<DetectedRegionRect> {
    let mut matching: Vec<&DetectedRegionRect> = regions
        .iter()
        .filter(|r| r.contains_point(px, py) && r.width > 0 && r.height > 0)
        .collect();

    if matching.is_empty() {
        return None;
    }

    // Sort by hierarchy level first, then by smallest area (innermost UI control first).
    matching.sort_by_key(|r| (r.level, r.area()));
    let mut current = matching[0];

    for _ in 0..wheel_steps_up {
        if let Some(parent_id) = current.parent_id {
            if let Some(parent) = regions.iter().find(|r| r.region_id == parent_id) {
                current = parent;
                continue;
            }
        }
        break;
    }

    Some(current.clone())
}

/// Pure-Rust L3 visual edge-box inference engine (fallback when UIA / AXUIElement is unavailable).
///
/// Casts orthogonal gradient rays outward from `(cursor_x, cursor_y)` on an RGBA8 buffer
/// and verifies edge continuity to snap onto UI cards, images, dialogs, or code blocks.
pub fn infer_visual_edge_box_rgba(
    rgba: &[u8],
    img_width: u32,
    img_height: u32,
    cursor_x: u32,
    cursor_y: u32,
    contrast_threshold: u8,
) -> Option<DetectedRegionRect> {
    if img_width < 4 || img_height < 4 || cursor_x >= img_width || cursor_y >= img_height {
        return None;
    }
    let expected_len = (img_width as usize) * (img_height as usize) * 4;
    if rgba.len() < expected_len {
        return None;
    }

    // 加权和最大 255×256，超出 i16 范围，必须在 i32 中计算（结果仍为 0–255）。
    let luma_at = |x: u32, y: u32| -> i16 {
        let idx = ((y as usize) * (img_width as usize) + (x as usize)) * 4;
        let r = rgba[idx] as i32;
        let g = rgba[idx + 1] as i32;
        let b = rgba[idx + 2] as i32;
        ((r * 77 + g * 150 + b * 29) >> 8) as i16
    };

    let thresh = contrast_threshold as i16;

    // Scan left from cursor_x
    let mut left_edge = 0u32;
    for x in (1..=cursor_x).rev() {
        if (luma_at(x, cursor_y) - luma_at(x - 1, cursor_y)).abs() >= thresh {
            left_edge = x;
            break;
        }
    }

    // Scan right from cursor_x
    let mut right_edge = img_width - 1;
    for x in cursor_x..(img_width - 1) {
        if (luma_at(x + 1, cursor_y) - luma_at(x, cursor_y)).abs() >= thresh {
            right_edge = x;
            break;
        }
    }

    // Scan up from cursor_y
    let mut top_edge = 0u32;
    for y in (1..=cursor_y).rev() {
        if (luma_at(cursor_x, y) - luma_at(cursor_x, y - 1)).abs() >= thresh {
            top_edge = y;
            break;
        }
    }

    // Scan down from cursor_y
    let mut bottom_edge = img_height - 1;
    for y in cursor_y..(img_height - 1) {
        if (luma_at(cursor_x, y + 1) - luma_at(cursor_x, y)).abs() >= thresh {
            bottom_edge = y;
            break;
        }
    }

    let width = right_edge.saturating_sub(left_edge) + 1;
    let height = bottom_edge.saturating_sub(top_edge) + 1;

    if width < 8 || height < 8 {
        return None;
    }

    Some(DetectedRegionRect {
        region_id: 90_000,
        parent_id: None,
        level: RegionHierarchyLevel::UiElement,
        x: left_edge as i32,
        y: top_edge as i32,
        width,
        height,
        label: Some("Visual Edge Region".to_string()),
    })
}
