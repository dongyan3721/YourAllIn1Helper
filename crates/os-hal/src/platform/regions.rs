//! 基于显示器和窗口边界的区域检测适配器。
use super::NativeOsHal;
#[cfg(not(target_os = "windows"))]
use crate::WindowController;
use crate::{HalResult, RegionDetector, ScreenCapturer};
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
            let parent_id = if cfg!(target_os = "windows") {
                regions
                    .iter()
                    .filter(|region| region.level == RegionHierarchyLevel::Monitor)
                    .find(|monitor| {
                        let center_x = window.x as i64 + window.width as i64 / 2;
                        let center_y = window.y as i64 + window.height as i64 / 2;
                        center_x >= monitor.x as i64
                            && center_y >= monitor.y as i64
                            && center_x < monitor.x as i64 + monitor.width as i64
                            && center_y < monitor.y as i64 + monitor.height as i64
                    })
                    .map(|monitor| monitor.region_id)
            } else {
                Some(1)
            };
            regions.push(DetectedRegionRect {
                region_id: 10_000 + index as u32,
                parent_id,
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

    #[cfg(target_os = "windows")]
    fn query_ui_hierarchy_at_point(&self, x: i32, y: i32) -> HalResult<Vec<DetectedRegionRect>> {
        super::windows::regions::query_hierarchy_at_point(x, y)
    }

    #[cfg(target_os = "macos")]
    fn check_ui_hierarchy_permission(&self) -> HalResult<()> {
        super::regions_ax::permission()
    }

    #[cfg(target_os = "windows")]
    fn check_ui_hierarchy_permission(&self) -> HalResult<()> {
        Ok(())
    }

    fn query_ui_element_at_point(&self, x: i32, y: i32) -> HalResult<Option<DetectedRegionRect>> {
        #[cfg(target_os = "windows")]
        {
            return super::windows::regions::query_hierarchy_at_point(x, y)
                .map(|regions| regions.into_iter().next());
        }
        #[cfg(not(target_os = "windows"))]
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
