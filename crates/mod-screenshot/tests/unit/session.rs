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

fn m1_caps() -> RegionCapabilities {
    RegionCapabilities::new(
        CapabilityState::Available,
        CapabilityState::Unavailable {
            reason: "需要辅助功能权限".into(),
        },
        CapabilityState::Disabled,
    )
}

fn session_with(
    detected: Vec<DetectedRegionRect>,
    rgba: Vec<u8>,
    caps: RegionCapabilities,
) -> CaptureSession {
    let frame = CapturedFrame {
        width: 800,
        height: 600,
        scale_factor_milli: 2000,
        rgba_bytes: rgba,
    };
    CaptureSession::new(geometry(), frame, detected, caps)
}

fn session(detected: Vec<DetectedRegionRect>) -> CaptureSession {
    session_with(detected, vec![0; 800 * 600 * 4], m1_caps())
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

fn ui(x: i32, y: i32, w: u32, h: u32, label: &str) -> DetectedRegionRect {
    DetectedRegionRect {
        region_id: 99,
        parent_id: None,
        level: RegionHierarchyLevel::UiElement,
        x,
        y,
        width: w,
        height: h,
        label: Some(label.into()),
    }
}

#[test]
fn ui_chain_climbs_leaf_container_window_monitor_and_is_clipped_to_window() {
    // 窗口 (50,50,100x100) 点 → (100,100,200x200) 像素。
    let s = session(vec![window(10, 50, 50, 100, 100, "editor")]);
    // 叶子按钮 (60,60,20x10) 点；容器超出窗口右侧，需裁剪。
    let chain = vec![ui(60, 60, 20, 10, "按钮"), ui(55, 55, 200, 40, "工具栏")];
    let leaf = s.resolve_region_with_ui(130, 130, 0, &chain).unwrap();
    assert_eq!(leaf.level, RegionHierarchyLevel::UiElement);
    assert_eq!(
        (leaf.x, leaf.y, leaf.width, leaf.height),
        (120, 120, 40, 20)
    );
    assert_eq!(leaf.label.as_deref(), Some("按钮"));
    let container = s.resolve_region_with_ui(130, 130, 1, &chain).unwrap();
    assert_eq!(container.level, RegionHierarchyLevel::ContainerPanel);
    assert_eq!(
        (container.x, container.y, container.width, container.height),
        (110, 110, 190, 80),
        "container must be clipped to the window"
    );
    assert_eq!(
        s.resolve_region_with_ui(130, 130, 2, &chain).unwrap().level,
        RegionHierarchyLevel::TopLevelWindow
    );
    assert_eq!(
        s.resolve_region_with_ui(130, 130, 3, &chain).unwrap().level,
        RegionHierarchyLevel::Monitor
    );
}

/// 黑底 800x600 帧中画一张白色卡片。
fn frame_with_card(x0: u32, y0: u32, w: u32, h: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; 800 * 600 * 4];
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let i = ((y * 800 + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    rgba
}

fn visual_caps() -> RegionCapabilities {
    RegionCapabilities::new(
        CapabilityState::Available,
        CapabilityState::Unavailable {
            reason: "需要辅助功能权限".into(),
        },
        CapabilityState::Available,
    )
}

#[test]
fn visual_edge_fallback_snaps_to_card_inside_window_when_ui_chain_is_empty() {
    let s = session_with(
        vec![window(10, 0, 0, 300, 250, "app")],
        frame_with_card(100, 100, 120, 80),
        visual_caps(),
    );
    let leaf = s.resolve_region_with_ui(150, 130, 0, &[]).unwrap();
    assert_eq!(leaf.label.as_deref(), Some("visual_edge"));
    assert_eq!(
        (leaf.x, leaf.y, leaf.width, leaf.height),
        (100, 100, 120, 80)
    );
    assert_eq!(
        s.resolve_region_with_ui(150, 130, 1, &[]).unwrap().level,
        RegionHierarchyLevel::TopLevelWindow
    );
}

#[test]
fn visual_edge_is_skipped_when_disabled_tiny_or_covering_the_window() {
    // 视觉候选关闭：直接命中窗口。
    let off = session_with(
        vec![window(10, 0, 0, 300, 250, "app")],
        frame_with_card(100, 100, 120, 80),
        m1_caps(),
    );
    assert_eq!(
        off.resolve_region_with_ui(150, 130, 0, &[]).unwrap().level,
        RegionHierarchyLevel::TopLevelWindow
    );
    // 卡片太小（文字笔画级别）不作为候选。
    let tiny = session_with(
        vec![window(10, 0, 0, 300, 250, "app")],
        frame_with_card(100, 100, 12, 12),
        visual_caps(),
    );
    assert_eq!(
        tiny.resolve_region_with_ui(105, 105, 0, &[]).unwrap().level,
        RegionHierarchyLevel::TopLevelWindow
    );
    // 有 L2 控件链时不再推导视觉候选。
    let with_ui = session_with(
        vec![window(10, 0, 0, 300, 250, "app")],
        frame_with_card(100, 100, 120, 80),
        visual_caps(),
    );
    let leaf = with_ui
        .resolve_region_with_ui(150, 130, 0, &[ui(70, 60, 20, 20, "按钮")])
        .unwrap();
    assert_eq!(leaf.label.as_deref(), Some("按钮"));
}
