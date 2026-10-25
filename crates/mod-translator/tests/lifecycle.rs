//! `mod-translator` 模块生命周期、窗口启停与快捷键集成测试。

use app_core::{AppKernelBuilder, AppModule, CoreResult, TargetOs, WindowBackend, WindowSpec};
use mod_translator::TranslatorModule;
use os_hal::MockOsHal;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tempfile::tempdir;

struct TestWindowBackend {
    opened_windows: Arc<parking_lot::RwLock<Vec<String>>>,
    closed_windows: Arc<parking_lot::RwLock<Vec<String>>>,
    fail_open: AtomicBool,
}

impl TestWindowBackend {
    fn new() -> Self {
        Self {
            opened_windows: Arc::new(parking_lot::RwLock::new(Vec::new())),
            closed_windows: Arc::new(parking_lot::RwLock::new(Vec::new())),
            fail_open: AtomicBool::new(false),
        }
    }
}

impl WindowBackend for TestWindowBackend {
    fn create_or_focus_window(&self, spec: &WindowSpec) -> CoreResult<()> {
        if self.fail_open.load(Ordering::SeqCst) {
            return Err(app_core::CoreError::Internal(anyhow::anyhow!(
                "模拟窗口创建失败"
            )));
        }
        self.opened_windows.write().push(spec.label.clone());
        Ok(())
    }

    fn destroy_window(&self, label: &str) -> CoreResult<()> {
        self.closed_windows.write().push(label.to_string());
        Ok(())
    }

    fn set_ignore_cursor_events(&self, _label: &str, _ignore: bool) -> CoreResult<()> {
        Ok(())
    }

    fn set_always_on_top(&self, _label: &str, _always_on_top: bool) -> CoreResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_translator_lifecycle_and_shutdown_cleanup() {
    let temp_dir = tempdir().unwrap();
    let window_backend = Arc::new(TestWindowBackend::new());
    let opened = window_backend.opened_windows.clone();
    let closed = window_backend.closed_windows.clone();

    let mock_hal = Arc::new(MockOsHal::new());
    let translator = Arc::new(TranslatorModule::new(mock_hal));

    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_in_memory_db(true)
        .with_target_os(TargetOs::current())
        .with_window_backend(window_backend)
        .register_module_arc(translator.clone())
        .unwrap()
        .bootstrap()
        .unwrap();

    // 1. 验证注册成功并处于激活运行态
    assert!(kernel.registry().is_enabled("translator"));
    assert!(translator.is_active());

    // 2. 模拟快捷键触发取词与弹窗
    let trigger_res = kernel.trigger_action("translator:translate_selection");
    assert!(trigger_res.is_ok());
    assert!(opened.read().contains(&"translate-popover".to_string()));

    // 3. 停用模块：触发 on_shutdown，验证窗口销毁与任务取消
    kernel.set_module_enabled("translator", false).unwrap();
    assert!(!translator.is_active());
    assert!(closed.read().contains(&"translate-popover".to_string()));

    // 4. 重复停用幂等性测试
    let ctx = kernel.context();
    let res_shutdown = translator.on_shutdown(ctx);
    assert!(res_shutdown.is_ok());

    // 5. 再次启用支持
    kernel.set_module_enabled("translator", true).unwrap();
    assert!(translator.is_active());
}
