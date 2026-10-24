use app_core::{AppKernelBuilder, TargetOs};
use mod_screen_ink::ScreenInkModule;
use os_hal::MockOsHal;
use std::sync::Arc;

#[test]
fn test_screen_ink_module_registration_and_disable_lifecycle() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mock = Arc::new(MockOsHal::new());
    let ink_module = Arc::new(ScreenInkModule::new(mock.clone()));

    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_target_os(TargetOs::MacOs)
        .register_module_arc(ink_module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();

    let ctx = kernel.context();

    // 验证初始状态与快捷键已就绪
    assert!(ctx
        .shortcut_registry
        .get_by_action("screen_ink:toggle_session")
        .is_some());
    assert!(ctx
        .shortcut_registry
        .get_by_action("screen_ink:toggle_passthrough")
        .is_some());

    // 打开会话
    ink_module.start_session(ctx).unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 2);

    // 禁用模块：触发 on_shutdown，自动销毁所有属于该模块的窗口
    kernel.set_module_enabled("screen_ink", false).unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 0);
    assert!(!ink_module.get_status().is_active);

    // 禁用状态下执行快捷键应被拦截
    let trigger_res = kernel.trigger_action("screen_ink:toggle_session");
    assert!(trigger_res.is_err());

    // 重新启用模块：支持再次初始化
    kernel.set_module_enabled("screen_ink", true).unwrap();
    assert!(ctx
        .shortcut_registry
        .get_by_action("screen_ink:toggle_session")
        .is_some());
    assert_eq!(ctx.window_pool.active_window_count(), 0);

    kernel.shutdown().unwrap();
}
