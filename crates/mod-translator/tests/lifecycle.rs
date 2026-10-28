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
    specs: parking_lot::RwLock<Vec<WindowSpec>>,
}

impl TestWindowBackend {
    fn new() -> Self {
        Self {
            opened_windows: Arc::new(parking_lot::RwLock::new(Vec::new())),
            closed_windows: Arc::new(parking_lot::RwLock::new(Vec::new())),
            fail_open: AtomicBool::new(false),
            specs: Default::default(),
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
        self.specs.write().push(spec.clone());
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

#[test]
fn test_translator_lifecycle_and_shutdown_cleanup() {
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
    let invocation = translator
        .take_selection()
        .expect("选区必须等气泡就绪后消费");
    assert_eq!(invocation.text.as_deref(), Some("Hello All-in-One"));
    assert!(translator.take_selection().is_none(), "同一选区仅消费一次");
    kernel
        .trigger_action("translator:translate_selection")
        .unwrap();
    assert!(
        translator.take_selection().is_some(),
        "已有气泡也应收到新选区"
    );
    kernel
        .trigger_action("translator:translate_selection")
        .unwrap();

    // 3. 停用模块：触发 on_shutdown，验证窗口销毁与任务取消
    kernel.set_module_enabled("translator", false).unwrap();
    assert!(!translator.is_active());
    assert!(translator.take_selection().is_none(), "停用必须清除原文");
    assert!(closed.read().contains(&"translate-popover".to_string()));

    // 4. 重复停用幂等性测试
    let ctx = kernel.context();
    let res_shutdown = translator.on_shutdown(ctx);
    assert!(res_shutdown.is_ok());

    // 5. 再次启用支持
    kernel.set_module_enabled("translator", true).unwrap();
    assert!(translator.is_active());
}

#[test]
fn denied_selection_is_delivered_after_window_opens_without_runtime() {
    struct DeniedSelection;
    impl os_hal::SelectionExtractor for DeniedSelection {
        fn is_available(&self) -> bool {
            true
        }
        fn extract_selected_text(&self) -> os_hal::HalResult<Option<os_hal::SelectionTextResult>> {
            Err(os_hal::HalError::PermissionDenied {
                capability: "accessibility",
                guidance: "Owned permission fixture".into(),
            })
        }
    }
    let dir = tempdir().unwrap();
    let backend = Arc::new(TestWindowBackend::new());
    let opened = backend.opened_windows.clone();
    let module = Arc::new(TranslatorModule::new(Arc::new(DeniedSelection)));
    let notified = Arc::new(AtomicBool::new(false));
    let flag = notified.clone();
    module.set_selection_sink(Arc::new(move || {
        assert!(opened.read().contains(&"translate-popover".into()));
        flag.store(true, Ordering::SeqCst);
    }));
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .with_window_backend(backend.clone())
        .register_module_arc(module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    kernel
        .trigger_action("translator:translate_selection")
        .unwrap();
    assert!(notified.load(Ordering::SeqCst));
    let state = module.take_selection().unwrap();
    assert!(state.permission_denied);
    assert!(state.text.is_none());
    assert_eq!(state.guidance.as_deref(), Some("Owned permission fixture"));
}

#[test]
fn mouse_selection_requires_opt_in_then_click_and_is_cleared_on_disable() {
    let dir = tempdir().unwrap();
    let backend = Arc::new(TestWindowBackend::new());
    let opened = backend.opened_windows.clone();
    let closed = backend.closed_windows.clone();
    let module = Arc::new(TranslatorModule::new(Arc::new(MockOsHal::new())));
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .with_target_os(TargetOs::current())
        .with_window_backend(backend.clone())
        .register_module_arc(module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    let selection = || os_hal::SelectionTextResult {
        text: "Owned mouse selection".into(),
        source: os_hal::SelectionSource::AccessibilityApi,
        source_app_name: Some("Fixture".into()),
    };
    let ctx = kernel.context();
    module
        .offer_mouse_selection(ctx, selection(), 12.0, 24.0)
        .unwrap();
    assert!(opened.read().is_empty(), "默认不开启鼠标选区读取入口");
    let mut cfg = module.config();
    cfg.mouse_selection_enabled = true;
    cfg.popover_width = 500.0;
    module.update_config(cfg).unwrap();
    module
        .offer_mouse_selection(ctx, selection(), 12.0, 24.0)
        .unwrap();
    assert_eq!(opened.read().as_slice(), &["translate-trigger"]);
    let trigger = backend.specs.read().last().unwrap().clone();
    assert!(!trigger.focus, "划词入口不得抢走目标应用焦点");
    assert_eq!((trigger.width, trigger.height), (20.0, 20.0));
    assert!(
        module.take_selection().is_none(),
        "点击前不得开始翻译或把选区交给页面"
    );
    module.open_mouse_selection(ctx).unwrap();
    assert_eq!(opened.read().last().unwrap(), "translate-popover");
    assert_eq!(backend.specs.read().last().unwrap().width, 500.0);
    assert_eq!(
        module.take_selection().unwrap().text.as_deref(),
        Some("Owned mouse selection")
    );
    assert!(
        module.open_mouse_selection(ctx).is_err(),
        "入口只能消费一次"
    );
    module
        .offer_mouse_selection(ctx, selection(), 12.0, 24.0)
        .unwrap();
    kernel.set_module_enabled("translator", false).unwrap();
    assert!(module.open_mouse_selection(ctx).is_err());
    assert!(module.take_selection().is_none());
    assert!(closed.read().contains(&"translate-trigger".into()));
    kernel.set_module_enabled("translator", true).unwrap();
    assert!(
        module.open_mouse_selection(ctx).is_err(),
        "重新启用不能恢复旧选区"
    );
}

#[test]
fn dismissed_or_failed_mouse_offer_cannot_translate_stale_selection() {
    let dir = tempdir().unwrap();
    let backend = Arc::new(TestWindowBackend::new());
    let module = Arc::new(TranslatorModule::new(Arc::new(MockOsHal::new())));
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .with_window_backend(backend.clone())
        .register_module_arc(module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    let mut cfg = module.config();
    cfg.mouse_selection_enabled = true;
    module.update_config(cfg).unwrap();
    let selection = || os_hal::SelectionTextResult {
        text: "Owned fixture".into(),
        source: os_hal::SelectionSource::AccessibilityApi,
        source_app_name: None,
    };
    let ctx = kernel.context();
    module
        .offer_mouse_selection(ctx, selection(), 0.0, 0.0)
        .unwrap();
    module.dismiss_mouse_selection(ctx).unwrap();
    assert!(module.open_mouse_selection(ctx).is_err());
    backend.fail_open.store(true, Ordering::SeqCst);
    assert!(module
        .offer_mouse_selection(ctx, selection(), 0.0, 0.0)
        .is_err());
    backend.fail_open.store(false, Ordering::SeqCst);
    assert!(module.open_mouse_selection(ctx).is_err());
    assert!(module.take_selection().is_none());
}
