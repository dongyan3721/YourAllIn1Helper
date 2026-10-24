use anyhow::anyhow;
use app_core::{AppKernelBuilder, CoreError, CoreResult, TargetOs, WindowBackend, WindowSpec};
use mod_screen_ink::ScreenInkModule;
use os_hal::MockOsHal;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

struct MockControlledWindowBackend {
    fail_toolbar_open: AtomicBool,
    fail_destroy: AtomicBool,
}

impl MockControlledWindowBackend {
    fn new() -> Self {
        Self {
            fail_toolbar_open: AtomicBool::new(false),
            fail_destroy: AtomicBool::new(false),
        }
    }
}

impl WindowBackend for MockControlledWindowBackend {
    fn create_or_focus_window(&self, spec: &WindowSpec) -> CoreResult<()> {
        if spec.label == "screen-ink-toolbar" && self.fail_toolbar_open.load(Ordering::SeqCst) {
            return Err(CoreError::Internal(anyhow!(
                "Simulated toolbar window creation failure"
            )));
        }
        Ok(())
    }

    fn destroy_window(&self, label: &str) -> CoreResult<()> {
        if self.fail_destroy.load(Ordering::SeqCst) {
            return Err(CoreError::Internal(anyhow!(
                "Simulated destroy failure for {}",
                label
            )));
        }
        Ok(())
    }

    fn set_ignore_cursor_events(&self, _label: &str, _ignore: bool) -> CoreResult<()> {
        Ok(())
    }

    fn set_always_on_top(&self, _label: &str, _always_on_top: bool) -> CoreResult<()> {
        Ok(())
    }

    fn set_window_visible(&self, _label: &str, _visible: bool) -> CoreResult<()> {
        Ok(())
    }
}

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

#[test]
fn test_screen_ink_start_session_rollback_on_failure() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mock_hal = Arc::new(MockOsHal::new());
    let ink_module = Arc::new(ScreenInkModule::new(mock_hal));

    let window_backend = Arc::new(MockControlledWindowBackend::new());
    // 设定第二窗口（toolbar）创建时失败
    window_backend
        .fail_toolbar_open
        .store(true, Ordering::SeqCst);

    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_target_os(TargetOs::MacOs)
        .with_window_backend(window_backend.clone())
        .register_module_arc(ink_module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();

    let ctx = kernel.context();

    // 1. 尝试启动：toolbar 创建失败，必须原子回滚销毁已创建的 canvas
    let start_res = ink_module.start_session(ctx);
    assert!(start_res.is_err());

    // 2. 验证状态与资源完全一致：无残留孤儿窗口，is_active 为 false
    assert_eq!(ctx.window_pool.active_window_count(), 0);
    assert!(ctx
        .window_pool
        .get_active_window("screen-ink-canvas")
        .is_none());
    assert!(!ink_module.get_status().is_active);

    // 3. 验证失败可重试：当故障恢复后，能够再次成功启动
    window_backend
        .fail_toolbar_open
        .store(false, Ordering::SeqCst);
    let retry_res = ink_module.start_session(ctx);
    assert!(retry_res.is_ok());
    assert_eq!(ctx.window_pool.active_window_count(), 2);
    assert!(ink_module.get_status().is_active);

    ink_module.exit_session(ctx).unwrap();
    kernel.shutdown().unwrap();
}

#[test]
fn test_screen_ink_exit_session_error_handling_and_retry() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mock_hal = Arc::new(MockOsHal::new());
    let ink_module = Arc::new(ScreenInkModule::new(mock_hal));

    let window_backend = Arc::new(MockControlledWindowBackend::new());
    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_target_os(TargetOs::MacOs)
        .with_window_backend(window_backend.clone())
        .register_module_arc(ink_module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();

    let ctx = kernel.context();

    // 1. 正常启动会话
    ink_module.start_session(ctx).unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 2);
    assert!(ink_module.get_status().is_active);

    // 2. 模拟销毁窗口失败
    window_backend.fail_destroy.store(true, Ordering::SeqCst);
    let exit_res = ink_module.exit_session(ctx);

    // 退出必须报错，且不能谎报已关闭
    assert!(exit_res.is_err());
    assert!(ink_module.get_status().is_active);
    assert!(ctx.window_pool.active_window_count() > 0);

    // 3. 故障恢复后重试退出 -> 成功销毁
    window_backend.fail_destroy.store(false, Ordering::SeqCst);
    let retry_exit = ink_module.exit_session(ctx);
    assert!(retry_exit.is_ok());
    assert_eq!(ctx.window_pool.active_window_count(), 0);
    assert!(!ink_module.get_status().is_active);

    kernel.shutdown().unwrap();
}
