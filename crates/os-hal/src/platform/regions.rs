//! 基于显示器和窗口边界的区域检测适配器。
use super::NativeOsHal;
use crate::{HalResult, RegionDetector, ScreenCapturer, WindowController};
use app_core::{DetectedRegionRect, RegionHierarchyLevel};

impl RegionDetector for NativeOsHal {
    fn detect_hierarchical_regions(&self) -> HalResult<Vec<DetectedRegionRect>> {
        let mut regions = Vec::new();
        for (index, monitor) in self.list_monitors()?.into_iter().enumerate() {
            regions.push(DetectedRegionRect {
                region_id: index as u32 + 1,
                parent_id: None,
                level: RegionHierarchyLevel::Monitor,
                x: monitor.x,
                y: monitor.y,
                width: monitor.width,
                height: monitor.height,
                label: Some(monitor.name),
            });
        }
        for (index, window) in self.native_windows()?.into_iter().enumerate() {
            regions.push(DetectedRegionRect {
                region_id: 10_000 + index as u32,
                parent_id: Some(1),
                level: RegionHierarchyLevel::TopLevelWindow,
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
                label: Some(format!("{} — {}", window.process_name, window.title)),
            });
        }
        Ok(regions)
    }

    #[cfg(target_os = "macos")]
    fn query_ui_hierarchy_at_point(&self, x: i32, y: i32) -> HalResult<Vec<DetectedRegionRect>> {
        self.ax_hierarchy_at_point(x, y)
    }

    #[cfg(target_os = "macos")]
    fn check_ui_hierarchy_permission(&self) -> HalResult<()> {
        super::regions_ax::permission()
    }

    fn query_ui_element_at_point(&self, x: i32, y: i32) -> HalResult<Option<DetectedRegionRect>> {
        Ok(self
            .get_window_at_point(x, y)
            .ok()
            .map(|window| DetectedRegionRect {
                region_id: window.handle as u32,
                parent_id: None,
                level: RegionHierarchyLevel::TopLevelWindow,
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
                label: Some(window.title),
            }))
    }
}
