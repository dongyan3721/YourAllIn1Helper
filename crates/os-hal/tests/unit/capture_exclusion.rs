//! 原生排除的纯逻辑与失败边界；不创建窗口、不抓取真实桌面。
use super::*;

#[test]
fn explicit_below_window_id_takes_precedence() {
    assert_eq!(resolve_capture_exclusion(42, Some(7)), Some(42));
    assert_eq!(resolve_capture_exclusion(0, Some(7)), Some(7));
    assert_eq!(resolve_capture_exclusion(0, None), None);
}

#[test]
fn clearing_an_unregistered_exclusion_is_safe() {
    let hal = NativeOsHal::new();
    hal.set_capture_exclusion(None).unwrap();
    assert_eq!(hal.active_capture_exclusion(), None);
}

#[cfg(target_os = "windows")]
#[test]
fn live_capture_without_registered_overlay_fails_before_recording() {
    let hal = NativeOsHal::new();
    let error = hal.prepare_live_capture().unwrap_err().to_string();
    assert!(error.contains("遮罩排除未就绪"));
    assert_eq!(hal.active_capture_exclusion(), None);
}

#[cfg(all(target_os = "windows", target_pointer_width = "64"))]
#[test]
fn windows_hwnd_keeps_all_64_bits() {
    let handle: WindowHandleId = 0x0000_7FFE_1234_5678;
    let hwnd = hwnd_from_window_handle(handle).expect("64-bit HWND must fit usize");
    assert_eq!(hwnd.0 as usize as WindowHandleId, handle);
    assert!(hwnd_from_window_handle(0).is_err());
}

#[cfg(target_os = "windows")]
#[test]
fn live_hole_uses_physical_pixels_on_negative_origin_monitors() {
    use windows::Win32::Foundation::RECT;
    let window = RECT {
        left: -2560,
        top: -120,
        right: 0,
        bottom: 1320,
    };
    let hole = live_hole_rect(
        SelectionBox {
            x: -2400,
            y: 40,
            width: 640,
            height: 480,
        },
        window,
    )
    .unwrap();
    assert_eq!(
        (hole.left, hole.top, hole.right, hole.bottom),
        (160, 160, 800, 640)
    );
    assert!(live_hole_rect(
        SelectionBox {
            x: -2561,
            y: 0,
            width: 640,
            height: 480
        },
        window
    )
    .is_err());
    assert!(live_hole_rect(
        SelectionBox {
            x: -100,
            y: 0,
            width: 640,
            height: 480
        },
        window
    )
    .is_err());
    assert!(live_hole_rect(
        SelectionBox {
            x: -2000,
            y: 0,
            width: 0,
            height: 480
        },
        window
    )
    .is_err());
}
