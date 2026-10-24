use super::*;
use app_core::{AppKernelBuilder, TargetOs};
use os_hal::MockOsHal;

#[test]
fn test_window_pin_toggle_and_auto_restore_on_shutdown() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mock_hal = MockOsHal::new();
    let winpin = WindowPinModule::new(Arc::new(mock_hal.clone()));

    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_in_memory_db(true)
        .with_target_os(TargetOs::Windows)
        .register_module(winpin.clone())
        .unwrap()
        .bootstrap()
        .unwrap();

    // 触发快捷键 PrimaryMod+Shift+P -> 置顶前台窗口 (句柄 1001)
    kernel
        .trigger_action("window_pin:toggle_foreground")
        .unwrap();
    assert!(mock_hal.get_window_info(1001).unwrap().is_topmost);
    assert_eq!(winpin.list_pinned_windows().len(), 1);

    // 置顶第二个窗口 (句柄 1002)
    winpin.pin_window(1002).unwrap();
    assert!(mock_hal.get_window_info(1002).unwrap().is_topmost);
    assert_eq!(winpin.list_pinned_windows().len(), 2);

    // 守护进程退出关闭 -> 必须自动将所有外部窗口还原为普通非置顶状态！
    kernel.shutdown().unwrap();
    assert!(!mock_hal.get_window_info(1001).unwrap().is_topmost);
    assert!(!mock_hal.get_window_info(1002).unwrap().is_topmost);
    assert_eq!(winpin.list_pinned_windows().len(), 0);
}
