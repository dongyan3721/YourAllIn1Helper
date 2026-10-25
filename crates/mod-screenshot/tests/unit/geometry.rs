//! 坐标换算、负坐标显示器与裁剪边界的单元测试。
use super::*;

fn monitor(x: i32, y: i32, w: u32, h: u32, scale: f64) -> DisplayMonitorInfo {
    DisplayMonitorInfo {
        id: 7,
        name: "Test Display".into(),
        x,
        y,
        width: w,
        height: h,
        scale_factor: scale,
        is_primary: x == 0 && y == 0,
    }
}

#[test]
fn retina_frame_maps_points_to_double_pixels() {
    // macOS：桌面单位为逻辑点，帧为 2 倍物理像素。
    let geo = MonitorGeometry::new(&monitor(0, 0, 1512, 982, 2.0), 3024, 1964).unwrap();
    let overlay = geo.overlay_logical_rect();
    assert_eq!((overlay.x, overlay.y), (0.0, 0.0));
    assert_eq!((overlay.width, overlay.height), (1512.0, 982.0));
    // 一个 100x50 点的窗口在帧中应为 200x100 像素。
    let rect = geo.desktop_rect_to_frame(10, 20, 100, 50).unwrap();
    assert_eq!(
        rect,
        SelectionBox {
            x: 20,
            y: 40,
            width: 200,
            height: 100
        }
    );
    // 帧像素选区换回逻辑点用于贴图摆放。
    let logical = geo.frame_to_logical(&rect);
    assert_eq!(
        (logical.x, logical.y, logical.width, logical.height),
        (10.0, 20.0, 100.0, 50.0)
    );
}

#[test]
fn windows_high_dpi_desktop_units_are_physical_pixels() {
    // Windows：桌面单位为物理像素，缩放 150% 时逻辑尺寸为物理 / 1.5。
    let geo = MonitorGeometry::new(&monitor(0, 0, 2880, 1620, 1.5), 2880, 1620).unwrap();
    let overlay = geo.overlay_logical_rect();
    assert_eq!((overlay.width, overlay.height), (1920.0, 1080.0));
    let rect = geo.desktop_rect_to_frame(300, 150, 600, 300).unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (300, 150, 600, 300)
    );
    let logical = geo.frame_to_logical(&rect);
    assert_eq!(
        (logical.x, logical.y, logical.width, logical.height),
        (200.0, 100.0, 400.0, 200.0)
    );
}

#[test]
fn negative_origin_secondary_display_is_relative_to_its_own_frame() {
    // 位于主屏左侧的副屏：原点为负坐标。
    let geo = MonitorGeometry::new(&monitor(-1920, -200, 1920, 1080, 2.0), 3840, 2160).unwrap();
    let overlay = geo.overlay_logical_rect();
    assert_eq!((overlay.x, overlay.y), (-1920.0, -200.0));
    // 跨越两块屏幕的窗口只保留落在本屏内的部分。
    let rect = geo.desktop_rect_to_frame(-100, -250, 400, 300).unwrap();
    assert_eq!(
        rect,
        SelectionBox {
            x: 3640,
            y: 0,
            width: 200,
            height: 500
        }
    );
    let logical = geo.frame_to_logical(&SelectionBox {
        x: 0,
        y: 0,
        width: 20,
        height: 20,
    });
    assert_eq!((logical.x, logical.y), (-1920.0, -200.0));
    // 完全位于其他屏幕的窗口不产生区域。
    assert!(geo.desktop_rect_to_frame(10, 10, 100, 100).is_none());
}

#[test]
fn invalid_scale_factor_falls_back_to_frame_ratio_and_zero_size_is_rejected() {
    let geo = MonitorGeometry::new(&monitor(0, 0, 1000, 500, f64::NAN), 2000, 1000).unwrap();
    assert_eq!(geo.device_pixel_ratio, 2.0);
    assert!(MonitorGeometry::new(&monitor(0, 0, 0, 500, 1.0), 10, 10).is_err());
    assert!(MonitorGeometry::new(&monitor(0, 0, 100, 100, 1.0), 0, 10).is_err());
}

#[test]
fn normalize_crop_clips_overflow_and_rejects_outside_or_empty() {
    let clipped = normalize_crop(
        &SelectionBox {
            x: -10,
            y: 90,
            width: 30,
            height: 30,
        },
        100,
        100,
    )
    .unwrap();
    assert_eq!(
        clipped,
        SelectionBox {
            x: 0,
            y: 90,
            width: 20,
            height: 10
        }
    );
    assert!(normalize_crop(
        &SelectionBox {
            x: 120,
            y: 0,
            width: 10,
            height: 10
        },
        100,
        100
    )
    .is_err());
    assert!(normalize_crop(
        &SelectionBox {
            x: 5,
            y: 5,
            width: 0,
            height: 10
        },
        100,
        100
    )
    .is_err());
}

#[test]
fn crop_rgba_copies_exact_rows_and_validates_bounds() {
    // 4x3 帧，每个像素的 R 通道编码为 y*10+x，便于检查裁剪位置。
    let (w, h) = (4u32, 3u32);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&[(y * 10 + x) as u8, 0, 0, 255]);
        }
    }
    let sel = SelectionBox {
        x: 1,
        y: 1,
        width: 2,
        height: 2,
    };
    let out = crop_rgba(&rgba, w, h, &sel).unwrap();
    let reds: Vec<u8> = out.chunks(4).map(|p| p[0]).collect();
    assert_eq!(reds, vec![11, 12, 21, 22]);
    // 越界与长度不匹配都必须报错，不能静默截断。
    let overflow = SelectionBox {
        x: 3,
        y: 0,
        width: 2,
        height: 1,
    };
    assert!(crop_rgba(&rgba, w, h, &overflow).is_err());
    assert!(crop_rgba(&rgba[..8], w, h, &sel).is_err());
}

#[test]
fn monitor_region_is_root_of_hierarchy() {
    let geo = MonitorGeometry::new(&monitor(0, 0, 800, 600, 1.0), 800, 600).unwrap();
    let root = geo.monitor_region(1);
    assert_eq!(root.level, RegionHierarchyLevel::Monitor);
    assert_eq!((root.width, root.height), (800, 600));
    assert!(root.parent_id.is_none());
}
