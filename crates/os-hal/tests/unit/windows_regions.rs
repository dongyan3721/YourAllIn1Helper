use super::*;

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> PhysicalRect {
    PhysicalRect {
        left,
        top,
        right,
        bottom,
    }
}

fn node(identity: u64, rect: PhysicalRect, label: &str) -> UiNode {
    UiNode {
        identity,
        rect,
        label: Some(label.into()),
    }
}

#[test]
fn hierarchy_trims_bounds_deduplicates_and_links_real_ancestors() {
    let root = node(1, rect(-1920, 0, 0, 1080), "window");
    let nodes = [
        node(2, rect(-20, 10, 40, 80), "leaf"),
        node(3, rect(-40, 0, 0, 90), "container"),
        node(4, rect(-40, 0, 0, 90), "duplicate bounds"),
        node(5, rect(-1920, 0, 0, 1080), "root duplicate"),
        node(2, rect(-100, 0, 0, 100), "cycle"),
    ];

    let regions = regions_from_chain(&nodes, &root).unwrap();
    assert_eq!(regions.len(), 3);
    assert_eq!(regions[0].level, RegionHierarchyLevel::UiElement);
    assert_eq!(regions[0].x, -20);
    assert_eq!(regions[0].width, 20);
    assert_eq!(regions[0].parent_id, Some(regions[1].region_id));
    assert_eq!(regions[1].level, RegionHierarchyLevel::ContainerPanel);
    assert_eq!(regions[1].parent_id, Some(regions[2].region_id));
    assert_eq!(regions[2].level, RegionHierarchyLevel::TopLevelWindow);
    assert_eq!(regions[2].parent_id, None);
}

#[test]
fn physical_rectangles_keep_negative_multi_monitor_coordinates_without_scaling_twice() {
    let monitor_left = rect(-1920, 0, 0, 1080);
    let dpi_scaled_window = rect(-1600, 120, -400, 900);
    let ui_element = rect(-1700, 100, -300, 930);
    let clipped = ui_element.intersection(dpi_scaled_window).unwrap();
    assert_eq!(clipped, rect(-1600, 120, -400, 900));
    assert!(monitor_left.contains(clipped.left, clipped.top));
    assert_eq!(clipped.region_size(), Some((1200, 780)));
}

#[test]
fn invalid_or_out_of_window_element_bounds_are_not_emitted() {
    let root = node(1, rect(0, 0, 100, 100), "window");
    let outside = [node(2, rect(120, 10, 140, 30), "outside")];
    assert!(matches!(
        regions_from_chain(&outside, &root),
        Err(HalError::Unsupported("windows_uiautomation_hit_element"))
    ));
    assert_eq!(
        PhysicalRect::from_win32(RECT {
            left: 10,
            top: 20,
            right: 10,
            bottom: 40,
        }),
        None
    );
}

#[test]
fn walker_result_keeps_success_null_distinct_from_hresult_failure() {
    assert_eq!(
        interface_result_has_value(HRESULT(0), true, "test").unwrap(),
        false
    );
    assert_eq!(
        interface_result_has_value(HRESULT(0), false, "test").unwrap(),
        true
    );
    assert!(matches!(
        interface_result_has_value(HRESULT(0x80004005u32 as i32), true, "test"),
        Err(HalError::NativeCallFailed(_))
    ));
}
