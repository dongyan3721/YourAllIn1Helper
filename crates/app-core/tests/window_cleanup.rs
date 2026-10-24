//! 窗口清理失败必须向生命周期调用者返回错误，并保留可重试的资源记录。
use app_core::{CoreError, CoreResult, WindowBackend, WindowPoolManager, WindowSpec};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
struct Backend {
    fail_once: AtomicBool,
}
impl WindowBackend for Backend {
    fn create_or_focus_window(&self, _: &WindowSpec) -> CoreResult<()> {
        Ok(())
    }
    fn destroy_window(&self, label: &str) -> CoreResult<()> {
        if self.fail_once.swap(false, Ordering::SeqCst) {
            Err(CoreError::WindowLifecycle {
                label: label.into(),
                reason: "destroy failed".into(),
            })
        } else {
            Ok(())
        }
    }
    fn set_ignore_cursor_events(&self, _: &str, _: bool) -> CoreResult<()> {
        Ok(())
    }
    fn set_always_on_top(&self, _: &str, _: bool) -> CoreResult<()> {
        Ok(())
    }
}
#[test]
fn failed_native_cleanup_keeps_window_tracked_until_retry_succeeds() {
    let pool = WindowPoolManager::new(Arc::new(Backend {
        fail_once: AtomicBool::new(true),
    }));
    pool.open_window(WindowSpec::clipboard_popup(0.0, 0.0))
        .unwrap();
    assert!(pool.destroy_window("clipboard-popup").is_err());
    assert_eq!(pool.active_window_count(), 1);
    pool.destroy_window("clipboard-popup").unwrap();
    assert_eq!(pool.active_window_count(), 0);
}
