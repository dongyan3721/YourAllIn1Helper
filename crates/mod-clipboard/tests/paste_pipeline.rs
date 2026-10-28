use app_core::{AppKernel, AppKernelBuilder, CoreResult, TargetOs, WindowBackend, WindowSpec};
use mod_clipboard::ClipboardModule;
use os_hal::{HalError, HalResult, InputSimulator};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Default)]
struct Probe {
    events: Mutex<Vec<&'static str>>,
    denied: AtomicBool,
    clipboard: Mutex<String>,
    pasted: Mutex<Option<String>>,
}
impl InputSimulator for Probe {
    fn remember_paste_target(&self) -> HalResult<()> {
        self.events.lock().unwrap().push("remember");
        Ok(())
    }
    fn check_input_permission(&self) -> HalResult<()> {
        self.events.lock().unwrap().push("check");
        if self.denied.load(Ordering::SeqCst) {
            return Err(HalError::PermissionDenied {
                capability: "accessibility",
                guidance: "authorize current signed app".into(),
            });
        }
        Ok(())
    }
    fn send_paste_shortcut(&self) -> HalResult<()> {
        self.events.lock().unwrap().push("paste");
        *self.pasted.lock().unwrap() = Some(self.clipboard.lock().unwrap().clone());
        Ok(())
    }
    fn send_copy_shortcut(&self) -> HalResult<()> {
        Ok(())
    }
    fn scroll_wheel_at(&self, _: i32, _: i32, _: i32) -> HalResult<()> {
        Ok(())
    }
}
struct Backend(Arc<Probe>);
impl WindowBackend for Backend {
    fn create_or_focus_window(&self, _: &WindowSpec) -> CoreResult<()> {
        self.0.events.lock().unwrap().push("open");
        Ok(())
    }
    fn destroy_window(&self, _: &str) -> CoreResult<()> {
        self.0.events.lock().unwrap().push("close");
        Ok(())
    }
    fn set_ignore_cursor_events(&self, _: &str, _: bool) -> CoreResult<()> {
        Ok(())
    }
    fn set_always_on_top(&self, _: &str, _: bool) -> CoreResult<()> {
        Ok(())
    }
}
fn fixture(path: &std::path::Path) -> (AppKernel, ClipboardModule, Arc<Probe>) {
    let probe = Arc::new(Probe::default());
    *probe.clipboard.lock().unwrap() = "current-beta".into();
    let kernel = AppKernelBuilder::new(path)
        .with_in_memory_db(true)
        .with_target_os(TargetOs::MacOs)
        .with_window_backend(Arc::new(Backend(probe.clone())))
        .register_module(ClipboardModule::new(probe.clone()))
        .unwrap()
        .bootstrap()
        .unwrap();
    kernel.trigger_action("clipboard:toggle_popup").unwrap();
    probe.events.lock().unwrap().clear();
    (kernel, ClipboardModule::new(probe.clone()), probe)
}

#[test]
fn opening_remembers_destination_before_focus_changes_but_closing_does_not_replace_it() {
    let dir = tempfile::tempdir().unwrap();
    let (kernel, _, probe) = fixture(dir.path());
    kernel.trigger_action("clipboard:toggle_popup").unwrap();
    kernel.trigger_action("clipboard:toggle_popup").unwrap();
    assert_eq!(
        *probe.events.lock().unwrap(),
        vec!["close", "remember", "open"]
    );
}
#[test]
fn selected_payload_is_written_before_popup_close_and_paste() {
    let dir = tempfile::tempdir().unwrap();
    let (kernel, module, probe) = fixture(dir.path());
    let record = module
        .capture_text(kernel.context(), "history-alpha", None)
        .unwrap()
        .unwrap();
    module
        .paste_item_with_prepare(kernel.context(), record.id, false, |selected| {
            probe.events.lock().unwrap().push("write");
            *probe.clipboard.lock().unwrap() = selected.content_text.clone().unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        *probe.events.lock().unwrap(),
        vec!["check", "write", "close", "paste"]
    );
    assert_eq!(
        probe.pasted.lock().unwrap().as_deref(),
        Some("history-alpha")
    );
    assert_eq!(kernel.context().window_pool.active_window_count(), 0);
}
#[test]
fn permission_denied_keeps_popup_and_clipboard_without_attempting_write() {
    let dir = tempfile::tempdir().unwrap();
    let (kernel, module, probe) = fixture(dir.path());
    let record = module
        .capture_text(kernel.context(), "history-alpha", None)
        .unwrap()
        .unwrap();
    probe.denied.store(true, Ordering::SeqCst);
    let error = module.paste_item_with_prepare(kernel.context(), record.id, false, |_| {
        panic!("write must not run")
    });
    assert!(error
        .unwrap_err()
        .to_string()
        .contains("authorize current signed app"));
    assert_eq!(*probe.events.lock().unwrap(), vec!["check"]);
    assert_eq!(*probe.clipboard.lock().unwrap(), "current-beta");
    assert_eq!(kernel.context().window_pool.active_window_count(), 1);
    assert_eq!(
        module.list_items(kernel.context(), None, 10).unwrap()[0].updated_at,
        record.updated_at
    );
}
#[test]
fn failed_clipboard_write_keeps_popup_and_never_sends_paste() {
    let dir = tempfile::tempdir().unwrap();
    let (kernel, module, probe) = fixture(dir.path());
    let record = module
        .capture_text(kernel.context(), "history-alpha", None)
        .unwrap()
        .unwrap();
    let error = module.paste_item_with_prepare(kernel.context(), record.id, false, |_| {
        probe.events.lock().unwrap().push("write");
        Err(app_core::CoreError::Config("write failed".into()))
    });
    assert!(error.is_err());
    assert_eq!(*probe.events.lock().unwrap(), vec!["check", "write"]);
    assert_eq!(kernel.context().window_pool.active_window_count(), 1);
    assert!(probe.pasted.lock().unwrap().is_none());
}
