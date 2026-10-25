//! 会话区域投影、遮挡处理与层级扩展的单元测试。
use super::*;
use os_hal::DisplayMonitorInfo;

fn geometry() -> MonitorGeometry {
    let info = DisplayMonitorInfo {
        id: 1,
        name: "Retina".into(),
        x: 0,
        y: 0,
        width: 400,
        height: 300,
        scale_factor: 2.0,
        is_primary: true,
    };
    MonitorGeometry::new(&info, 800, 600).unwrap()
}

fn window(id: u32, x: i32, y: i32, w: u32, h: u32, label: &str) -> DetectedRegionRect {
    DetectedRegionRect {
        region_id: id,
        parent_id: Some(1),
        level: RegionHierarchyLevel::TopLevelWindow,
        x,
        y,
        width: w,
        height: h,
        label: Some(label.into()),
    }
}

fn session(detected: Vec<DetectedRegionRect>) -> CaptureSession {
    let frame = CapturedFrame {
        width: 800,
        height: 600,
        scale_factor_milli: 2000,
        rgba_bytes: vec![0; 800 * 600 * 4],
    };
    CaptureSession::new(
        geometry(),
        frame,
        detected,
        RegionCapabilities::m1(CapabilityState::Available),
    )
}

#[test]
fn projection_keeps_only_visible_top_level_windows_in_frame_pixels() {
    let monitor_entry = DetectedRegionRect {
        region_id: 1,
        parent_id: None,
        level: RegionHierarchyLevel::Monitor,
        x: 0,
        y: 0,
        width: 400,
        height: 300,
        label: None,
    };
    let s = session(vec![
        monitor_entry,
        window(10, 10, 10, 100, 50, "front"),
        window(11, 1000, 1000, 100, 100, "other display"),
        window(12, 0, 0, 2, 2, "shadow"),
    ]);
    assert_eq!(s.windows.len(), 1);
    let w = &s.windows[0];
    assert_eq!((w.x, w.y, w.width, w.height), (20, 20, 200, 100));
    assert_eq!(w.parent_id, Some(MONITOR_REGION_ID));
    assert_eq!(w.label.as_deref(), Some("front"));
}

#[test]
fn frontmost_window_wins_over_smaller_occluded_window() {
    // HAL 顺序：大窗口在前，小窗口在后被完全遮挡。
    let s = session(vec![
        window(10, 0, 0, 300, 200, "front big"),
        window(11, 50, 50, 40, 40, "occluded small"),
    ]);
    let hit = s.resolve_region(140, 140, 0).unwrap();
    assert_eq!(hit.label.as_deref(), Some("front big"));
}

#[test]
fn wheel_steps_climb_from_window_to_monitor_and_stop_at_root() {
    let s = session(vec![window(10, 50, 50, 100, 100, "editor")]);
    assert_eq!(
        s.resolve_region(150, 150, 0).unwrap().level,
        RegionHierarchyLevel::TopLevelWindow
    );
    let parent = s.resolve_region(150, 150, 1).unwrap();
    assert_eq!(parent.level, RegionHierarchyLevel::Monitor);
    assert_eq!((parent.width, parent.height), (800, 600));
    assert_eq!(
        s.resolve_region(150, 150, 5).unwrap().level,
        RegionHierarchyLevel::Monitor
    );
    // 桌面空白处直接命中显示器；帧外坐标没有区域。
    assert_eq!(
        s.resolve_region(10, 10, 0).unwrap().level,
        RegionHierarchyLevel::Monitor
    );
    assert!(s.resolve_region(-1, 10, 0).is_none());
}

#[test]
fn describe_reports_pixels_logical_size_and_honest_capabilities() {
    let s = session(vec![]);
    let info = s.describe(OverlaySettings {
        manual_selection: true,
        smart_regions: true,
        wheel_expand: true,
        magnifier: true,
        default_color: "#ff3b30".into(),
        default_stroke_width: 4.0,
    });
    assert_eq!((info.frame_width, info.frame_height), (800, 600));
    assert_eq!(info.frame_byte_length, 800 * 600 * 4);
    assert_eq!((info.logical_width, info.logical_height), (400.0, 300.0));
    assert!(matches!(
        info.capabilities.ui_elements,
        CapabilityState::Unavailable { .. }
    ));
    let json = serde_json::to_value(&info.capabilities).unwrap();
    assert_eq!(json["window_tree"]["state"], "available");
    assert_eq!(json["ui_elements"]["state"], "unavailable");
}
