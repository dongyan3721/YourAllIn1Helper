use crate::{InputSimulator, MockOsHal, NativeOsHal, WindowController};

#[test]
fn test_mock_os_hal_window_topmost_and_paste() {
    let hal = MockOsHal::new();
    let fg = hal.get_foreground_window().unwrap();
    assert_eq!(fg.handle, 1001);
    assert!(!fg.is_topmost);

    hal.set_external_window_topmost(1001, true).unwrap();
    assert!(hal.get_window_info(1001).unwrap().is_topmost);

    hal.send_paste_shortcut().unwrap();
    assert_eq!(hal.paste_trigger_count(), 1);
}

#[test]
fn test_selection_extractor_contracts() {
    use crate::SelectionExtractor;

    // 1. MockOsHal 测试取词能力
    let mock = MockOsHal::new();
    assert!(mock.is_available());
    let res = mock.extract_selected_text().unwrap();
    assert!(res.is_some());
    let selection = res.unwrap();
    assert_eq!(selection.text, "Hello All-in-One");
    assert_eq!(selection.source, crate::SelectionSource::AccessibilityApi);

    // 2. NativeOsHal 适配器可用性与接口行为
    let native = NativeOsHal::new();
    #[cfg(target_os = "macos")]
    assert!(native.is_available());

    #[cfg(not(target_os = "macos"))]
    assert!(!native.is_available());
    // 原生取词留给显式桌面验收，单元测试不得读取真实前台应用。
}

#[test]
#[ignore = "requires a logged-in desktop session"]
fn test_native_windows_list() {
    let hal = NativeOsHal::new();
    let windows = hal.list_external_windows().unwrap();
    // 校验：必须绝不包含 Window Server、Dock 等系统合成服务
    for win in &windows {
        assert_ne!(
            win.process_name, "Window Server",
            "严禁将 Window Server 列入普通应用候选窗口"
        );
        assert_ne!(win.process_name, "Dock", "严禁将 Dock 列入普通应用候选窗口");
        assert!(win.width >= 32, "窗口宽度必须合规");
        assert!(win.height >= 32, "窗口高度必须合规");
        assert!(win.handle > 0, "窗口句柄必须合法有效");
    }
}
