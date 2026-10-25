//! 模块私有辅助函数的单元测试。
use super::*;

fn monitor(id: u32, primary: bool) -> DisplayMonitorInfo {
    DisplayMonitorInfo {
        id,
        name: format!("M{}", id),
        x: 0,
        y: 0,
        width: 100,
        height: 100,
        scale_factor: 1.0,
        is_primary: primary,
    }
}

#[test]
fn pick_monitor_prefers_requested_then_primary_then_first() {
    let list = vec![monitor(3, false), monitor(5, true)];
    assert_eq!(pick_monitor(&list, Some(3)).unwrap().id, 3);
    assert_eq!(pick_monitor(&list, None).unwrap().id, 5);
    assert!(pick_monitor(&list, Some(9)).is_none());
    assert_eq!(pick_monitor(&[monitor(8, false)], None).unwrap().id, 8);
    assert!(pick_monitor(&[], None).is_none());
}

#[test]
fn rgb_text_validation_accepts_only_color_values() {
    assert!(is_rgb_text("rgb(0, 128, 255)"));
    assert!(is_rgb_text("rgb(1,2,3)"));
    assert!(!is_rgb_text("rgb(256, 0, 0)"));
    assert!(!is_rgb_text("rgb(1, 2)"));
    assert!(!is_rgb_text("hello"));
}

#[test]
fn only_implemented_capture_shortcut_is_declared() {
    let module = ScreenshotModule::new(
        Arc::new(os_hal::MockOsHal::new()),
        Arc::new(os_hal::MockOsHal::new()),
        Arc::new(clipboard::UnavailableClipboard),
    );
    let shortcuts = module.default_shortcuts();
    assert_eq!(shortcuts.len(), 4);
    assert_eq!(shortcuts[0].action_id, ACTION_CAPTURE);
    assert_eq!(shortcuts[0].default_logical_hotkey, "F1");
    assert_eq!(shortcuts[1].action_id, ACTION_SCROLL);
    assert_eq!(shortcuts[1].default_logical_hotkey, "Shift+F1");
    assert_eq!(shortcuts[2].action_id, ACTION_GIF);
    assert_eq!(shortcuts[2].default_logical_hotkey, "SecondaryMod+F1");
    assert_eq!(shortcuts[3].action_id, ACTION_OCR);
    assert_eq!(shortcuts[3].default_logical_hotkey, "PrimaryMod+Shift+O");
    assert_eq!(module.id(), "screenshot");
}
