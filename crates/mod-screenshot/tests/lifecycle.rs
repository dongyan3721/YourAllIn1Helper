//! 截图模块生命周期集成测试。
//!
//! 使用显式注入的测试适配器（自制渐变帧、记录型窗口后端与剪贴板），不读取真实屏幕、
//! 不写系统剪贴板；所有文件写入临时目录。
use anyhow::anyhow;
use app_core::{
    AppKernel, AppKernelBuilder, AppModule, CoreError, CoreResult, DetectedRegionRect,
    RegionHierarchyLevel, SelectionBox, ShortcutSpec, TargetOs, WindowBackend, WindowSpec,
};
use mod_screenshot::{
    clipboard::ScreenshotClipboard, export::ExportAction, ScreenshotModule, ACTION_CAPTURE,
    OVERLAY_LABEL,
};
use os_hal::{
    CapturedFrame, DisplayMonitorInfo, HalError, HalResult, RegionDetector, ScreenCapturer,
};
use parking_lot::Mutex;
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

/// 事件日志：验证「先抓帧、后建遮罩」等顺序约束。
type EventLog = Arc<Mutex<Vec<String>>>;

/// 自制 Retina 屏：桌面 40x30 点，帧 80x60 像素。
struct FakeScreen {
    log: EventLog,
    captures: AtomicUsize,
    deny_permission: AtomicBool,
    permission_requests: AtomicUsize,
    fail_regions: AtomicBool,
    /// 模拟已授予辅助功能权限，L2 返回固定控件链。
    ui_trusted: AtomicBool,
}

impl FakeScreen {
    fn new(log: EventLog) -> Self {
        Self {
            log,
            captures: AtomicUsize::new(0),
            deny_permission: AtomicBool::new(false),
            permission_requests: AtomicUsize::new(0),
            fail_regions: AtomicBool::new(false),
            ui_trusted: AtomicBool::new(false),
        }
    }
}

impl ScreenCapturer for FakeScreen {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        Ok(vec![DisplayMonitorInfo {
            id: 42,
            name: "Fake Retina".into(),
            x: 0,
            y: 0,
            width: 40,
            height: 30,
            scale_factor: 2.0,
            is_primary: true,
        }])
    }

    fn capture_monitor(&self, monitor_id: Option<u32>) -> HalResult<CapturedFrame> {
        assert_eq!(monitor_id, Some(42));
        self.captures.fetch_add(1, Ordering::SeqCst);
        self.log.lock().push("capture".into());
        let (w, h) = (80u32, 60u32);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[x as u8, y as u8, 7, 255]);
            }
        }
        Ok(CapturedFrame {
            width: w,
            height: h,
            scale_factor_milli: 2000,
            rgba_bytes: rgba,
        })
    }

    fn capture_region(&self, _bounds: SelectionBox) -> HalResult<CapturedFrame> {
        Err(HalError::Unsupported("not used by screenshot module"))
    }

    fn request_capture_permission(&self) -> HalResult<()> {
        self.permission_requests.fetch_add(1, Ordering::SeqCst);
        self.check_capture_permission()
    }

    fn check_capture_permission(&self) -> HalResult<()> {
        if self.deny_permission.load(Ordering::SeqCst) {
            Err(HalError::PermissionDenied {
                capability: "screen_recording",
                guidance: "打开系统设置授权后重启".into(),
            })
        } else {
            Ok(())
        }
    }
}

impl RegionDetector for FakeScreen {
    fn detect_hierarchical_regions(&self) -> HalResult<Vec<DetectedRegionRect>> {
        if self.fail_regions.load(Ordering::SeqCst) {
            return Err(HalError::NativeCallFailed("window list unavailable".into()));
        }
        Ok(vec![DetectedRegionRect {
            region_id: 5,
            parent_id: Some(1),
            level: RegionHierarchyLevel::TopLevelWindow,
            x: 5,
            y: 5,
            width: 20,
            height: 10,
            label: Some("Editor".into()),
        }])
    }

    fn query_ui_element_at_point(&self, _x: i32, _y: i32) -> HalResult<Option<DetectedRegionRect>> {
        Err(HalError::Unsupported("ui_element_query"))
    }

    fn check_ui_hierarchy_permission(&self) -> HalResult<()> {
        if self.ui_trusted.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(HalError::PermissionDenied {
                capability: "accessibility",
                guidance: "请在系统设置中允许辅助功能".into(),
            })
        }
    }

    fn query_ui_hierarchy_at_point(&self, x: i32, y: i32) -> HalResult<Vec<DetectedRegionRect>> {
        // 桌面单位：窗口 (5,5,20x10) 内的按钮 (8,8,6x4)。
        self.log.lock().push(format!("ax:{},{}", x, y));
        Ok(vec![DetectedRegionRect {
            region_id: 1,
            parent_id: None,
            level: RegionHierarchyLevel::UiElement,
            x: 8,
            y: 8,
            width: 6,
            height: 4,
            label: Some("按钮".into()),
        }])
    }
}

/// 记录型窗口后端：保存创建、销毁、显隐事件。
struct RecordingBackend {
    log: EventLog,
    fail_create_prefix: Mutex<Option<String>>,
}

impl WindowBackend for RecordingBackend {
    fn create_or_focus_window(&self, spec: &WindowSpec) -> CoreResult<()> {
        if let Some(prefix) = self.fail_create_prefix.lock().as_ref() {
            if spec.label.starts_with(prefix.as_str()) {
                return Err(CoreError::Internal(anyhow!("simulated create failure")));
            }
        }
        self.log.lock().push(format!(
            "create:{}@{},{},{}x{}",
            spec.label,
            spec.x.unwrap_or(-1.0),
            spec.y.unwrap_or(-1.0),
            spec.width,
            spec.height
        ));
        Ok(())
    }

    fn destroy_window(&self, label: &str) -> CoreResult<()> {
        self.log.lock().push(format!("destroy:{}", label));
        Ok(())
    }

    fn set_ignore_cursor_events(&self, _label: &str, _ignore: bool) -> CoreResult<()> {
        Ok(())
    }

    fn set_always_on_top(&self, _label: &str, _always_on_top: bool) -> CoreResult<()> {
        Ok(())
    }

    fn set_window_visible(&self, label: &str, visible: bool) -> CoreResult<()> {
        self.log
            .lock()
            .push(format!("visible:{}={}", label, visible));
        Ok(())
    }
}

/// 记录型剪贴板。
#[derive(Default)]
struct RecordingClipboard {
    images: Mutex<Vec<(u32, u32, usize)>>,
    texts: Mutex<Vec<String>>,
    fail: AtomicBool,
}

impl ScreenshotClipboard for RecordingClipboard {
    fn write_image(&self, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("clipboard busy".into());
        }
        self.images.lock().push((width, height, rgba.len()));
        Ok(())
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        self.texts.lock().push(text.to_string());
        Ok(())
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    data_dir: std::path::PathBuf,
    log: EventLog,
    screen: Arc<FakeScreen>,
    clipboard: Arc<RecordingClipboard>,
    backend: Arc<RecordingBackend>,
    module: Arc<ScreenshotModule>,
    kernel: AppKernel,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let log: EventLog = Arc::new(Mutex::new(Vec::new()));
    let screen = Arc::new(FakeScreen::new(log.clone()));
    let clipboard = Arc::new(RecordingClipboard::default());
    let backend = Arc::new(RecordingBackend {
        log: log.clone(),
        fail_create_prefix: Mutex::new(None),
    });
    let module = Arc::new(ScreenshotModule::new(
        screen.clone(),
        screen.clone(),
        clipboard.clone(),
    ));
    let kernel = boot(&data_dir, backend.clone(), module.clone(), None);
    Harness {
        _dir: dir,
        data_dir,
        log,
        screen,
        clipboard,
        backend,
        module,
        kernel,
    }
}

fn boot(
    data_dir: &std::path::Path,
    backend: Arc<RecordingBackend>,
    module: Arc<ScreenshotModule>,
    extra: Option<Arc<dyn AppModule>>,
) -> AppKernel {
    let mut builder = AppKernelBuilder::new(data_dir)
        .with_in_memory_db(true)
        .with_target_os(TargetOs::MacOs)
        .with_window_backend(backend);
    if let Some(extra) = extra {
        builder = builder.register_module_arc(extra).unwrap();
    }
    builder
        .register_module_arc(module)
        .unwrap()
        .bootstrap()
        .unwrap()
}

fn events(h: &Harness) -> Vec<String> {
    h.log.lock().clone()
}

#[test]
fn init_does_not_capture_or_create_windows() {
    let h = harness();
    assert_eq!(h.screen.captures.load(Ordering::SeqCst), 0);
    assert_eq!(h.kernel.context().window_pool.active_window_count(), 0);
    assert!(!h.module.has_active_session());
    assert!(h
        .kernel
        .context()
        .shortcut_registry
        .get_by_action(ACTION_CAPTURE)
        .is_some());
}

#[test]
fn frozen_frame_is_captured_before_hidden_overlay_is_created_and_shown_once() {
    let h = harness();
    h.kernel.trigger_action(ACTION_CAPTURE).unwrap();
    let ev = events(&h);
    let capture_at = ev.iter().position(|e| e == "capture").unwrap();
    let create_at = ev
        .iter()
        .position(|e| e.starts_with("create:capture-overlay"))
        .unwrap();
    assert!(
        capture_at < create_at,
        "frame must be frozen before overlay: {ev:?}"
    );
    // 遮罩覆盖整块显示器的逻辑尺寸（80x60 像素 / 2）。
    assert!(ev[create_at].ends_with("@0,0,40x30"), "{}", ev[create_at]);

    let info = h.module.session_info(h.kernel.context(), None).unwrap();
    assert_eq!((info.frame_width, info.frame_height), (80, 60));
    assert_eq!(info.window_region_count, 1);
    h.module
        .overlay_ready(h.kernel.context(), &info.session_id)
        .unwrap();
    h.module
        .overlay_ready(h.kernel.context(), &info.session_id)
        .unwrap();
    let shows = events(&h)
        .iter()
        .filter(|e| *e == "visible:capture-overlay=true")
        .count();
    assert_eq!(shows, 1);
    assert_eq!(
        h.module.frame_bytes(&info.session_id).unwrap().len(),
        80 * 60 * 4
    );
}

#[test]
fn smart_region_uses_frame_pixels_and_detector_failure_degrades_honestly() {
    let h = harness();
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    // 窗口 (5,5,20x10) 点 → (10,10,40x20) 像素。
    let hit = h
        .module
        .resolve_region(&info.session_id, 20, 15, 0)
        .unwrap()
        .unwrap();
    assert_eq!((hit.x, hit.y, hit.width, hit.height), (10, 10, 40, 20));
    let parent = h
        .module
        .resolve_region(&info.session_id, 20, 15, 1)
        .unwrap()
        .unwrap();
    assert_eq!(parent.level, RegionHierarchyLevel::Monitor);

    h.screen.fail_regions.store(true, Ordering::SeqCst);
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    let caps = serde_json::to_value(&info.capabilities).unwrap();
    assert_eq!(caps["window_tree"]["state"], "unavailable");
    assert!(caps["window_tree"]["reason"]
        .as_str()
        .unwrap()
        .contains("window list unavailable"));
    assert_eq!(info.window_region_count, 0);
}

#[test]
fn cancel_releases_frame_and_destroys_overlay_and_stale_cancel_is_ignored() {
    let h = harness();
    let first = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    // 重复触发以新帧为准，旧会话 ID 立即失效。
    let second = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    assert_ne!(first.session_id, second.session_id);
    assert!(h.module.frame_bytes(&first.session_id).is_err());
    assert!(!h
        .module
        .cancel_capture(h.kernel.context(), Some(&first.session_id))
        .unwrap());
    assert!(h.module.has_active_session());

    assert!(h
        .module
        .cancel_capture(h.kernel.context(), Some(&second.session_id))
        .unwrap());
    assert!(!h.module.has_active_session());
    assert!(h.module.frame_bytes(&second.session_id).is_err());
    assert!(h
        .kernel
        .context()
        .window_pool
        .get_active_window(OVERLAY_LABEL)
        .is_none());
    assert!(events(&h).contains(&format!("destroy:{}", OVERLAY_LABEL)));
}

#[test]
fn copy_export_writes_selection_sized_image_and_finishes_session() {
    let h = harness();
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    let out = h
        .module
        .export(
            h.kernel.context(),
            &info.session_id,
            SelectionBox {
                x: 70,
                y: 50,
                width: 30,
                height: 30,
            },
            None,
            ExportAction::Copy,
        )
        .unwrap();
    // 超出帧的部分被裁掉：(70,50)-(80,60)。
    assert_eq!((out.width, out.height), (10, 10));
    assert!(out.copied);
    assert_eq!(h.clipboard.images.lock().as_slice(), &[(10, 10, 400)]);
    assert!(!h.module.has_active_session());
    assert_eq!(h.kernel.context().window_pool.active_window_count(), 0);
}

#[test]
fn copy_failure_keeps_session_and_reshows_overlay_for_retry() {
    let h = harness();
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    h.module
        .overlay_ready(h.kernel.context(), &info.session_id)
        .unwrap();
    h.clipboard.fail.store(true, Ordering::SeqCst);
    let sel = SelectionBox {
        x: 0,
        y: 0,
        width: 4,
        height: 4,
    };
    let err = h
        .module
        .export(
            h.kernel.context(),
            &info.session_id,
            sel,
            None,
            ExportAction::Copy,
        )
        .unwrap_err();
    assert!(err.to_string().contains("clipboard busy"));
    assert!(h.module.has_active_session());
    let ev = events(&h);
    assert_eq!(ev.last().unwrap(), "visible:capture-overlay=true");
    // 改为保存仍可完成。
    h.clipboard.fail.store(false, Ordering::SeqCst);
    let saved = h
        .module
        .export(
            h.kernel.context(),
            &info.session_id,
            sel,
            None,
            ExportAction::Save,
        )
        .unwrap();
    assert!(saved.saved_path.is_some());
}

#[test]
fn save_export_never_overwrites_and_annotations_are_composited() {
    let h = harness();
    let save_dir = h.data_dir.join("my shots");
    h.kernel
        .update_config(|c| {
            c.modules.settings.insert(
                "screenshot".into(),
                json!({"save_directory": save_dir, "file_name_prefix": "cap"}),
            );
        })
        .unwrap();
    let sel = SelectionBox {
        x: 2,
        y: 3,
        width: 4,
        height: 2,
    };
    // 标注层：左上角一个不透明红点，其余透明。
    let mut layer = image::RgbaImage::new(4, 2);
    layer.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    let mut png = Vec::new();
    layer
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let mut paths = Vec::new();
    for _ in 0..2 {
        let info = h
            .module
            .start_capture(h.kernel.context(), None, false)
            .unwrap();
        let out = h
            .module
            .export(
                h.kernel.context(),
                &info.session_id,
                sel,
                Some(&png),
                ExportAction::Save,
            )
            .unwrap();
        assert!(!out.copied);
        paths.push(std::path::PathBuf::from(out.saved_path.unwrap()));
    }
    assert_ne!(paths[0], paths[1]);
    for path in &paths {
        assert!(path.starts_with(&save_dir));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("cap_"));
        let img = image::open(path).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (4, 2));
        assert_eq!(img.get_pixel(0, 0), &image::Rgba([255, 0, 0, 255]));
        assert_eq!(img.get_pixel(1, 0), &image::Rgba([3, 3, 7, 255]));
    }
}

#[test]
fn pin_export_creates_resident_window_at_capture_position_and_close_frees_it() {
    let h = harness();
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    let out = h
        .module
        .export(
            h.kernel.context(),
            &info.session_id,
            SelectionBox {
                x: 20,
                y: 10,
                width: 16,
                height: 8,
            },
            None,
            ExportAction::Pin,
        )
        .unwrap();
    let pin = out.pin.unwrap();
    assert_eq!((pin.pixel_width, pin.pixel_height), (16, 8));
    assert_eq!((pin.logical_width, pin.logical_height), (8.0, 4.0));
    let label = format!("pin-img-{}", pin.pin_id);
    // 遮罩已销毁，只剩贴图窗口，位置为选区左上角的逻辑坐标 (10,5)。
    assert!(events(&h)
        .iter()
        .any(|e| e == &format!("create:{}@10,5,16x16", label)));
    assert_eq!(h.kernel.context().window_pool.active_window_count(), 1);
    let png = h.module.pin_png(&pin.pin_id).unwrap();
    let decoded = image::load_from_memory(&png).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (16, 8));

    h.module.copy_pin(&pin.pin_id).unwrap();
    assert_eq!(h.clipboard.images.lock().last().unwrap(), &(16, 8, 512));
    assert!(h.module.close_pin(h.kernel.context(), &pin.pin_id).unwrap());
    assert!(h.module.pin_png(&pin.pin_id).is_err());
    assert_eq!(h.kernel.context().window_pool.active_window_count(), 0);
    assert!(h
        .module
        .close_pin(h.kernel.context(), "zz-invalid")
        .is_err());
}

#[test]
fn pin_window_failure_rolls_back_record_and_keeps_session() {
    let h = harness();
    *h.backend.fail_create_prefix.lock() = Some("pin-img-".into());
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    let err = h.module.export(
        h.kernel.context(),
        &info.session_id,
        SelectionBox {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        },
        None,
        ExportAction::Pin,
    );
    assert!(err.is_err());
    assert!(h.module.list_pins().is_empty());
    assert!(h.module.has_active_session());
}

#[test]
fn permission_denied_reports_guidance_without_overlay_and_shortcut_opens_dashboard() {
    let h = harness();
    h.screen.deny_permission.store(true, Ordering::SeqCst);
    let err = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("screen_recording"), "{err}");
    assert!(err.contains("打开系统设置授权后重启"), "{err}");
    assert_eq!(h.screen.captures.load(Ordering::SeqCst), 0);
    assert!(h
        .kernel
        .context()
        .window_pool
        .get_active_window(OVERLAY_LABEL)
        .is_none());
    let status = h.module.status(h.kernel.context());
    assert!(!status.permission.granted);
    assert_eq!(
        serde_json::to_value(&status.last_error.unwrap()).unwrap()["kind"],
        "permission_denied"
    );

    assert!(h.kernel.trigger_action(ACTION_CAPTURE).is_err());
    let dashboard = h
        .kernel
        .context()
        .window_pool
        .get_active_window("dashboard")
        .unwrap();
    assert_eq!(dashboard.spec.route, "/dashboard/screenshot");
}

#[test]
fn disabling_module_destroys_overlay_and_pins_and_reenable_works() {
    let h = harness();
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    let pinned = h
        .module
        .export(
            h.kernel.context(),
            &info.session_id,
            SelectionBox {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            None,
            ExportAction::Pin,
        )
        .unwrap();
    h.module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    assert_eq!(h.kernel.context().window_pool.active_window_count(), 2);

    h.kernel.set_module_enabled("screenshot", false).unwrap();
    assert_eq!(h.kernel.context().window_pool.active_window_count(), 0);
    assert!(!h.module.has_active_session());
    assert!(h.module.list_pins().is_empty());
    assert!(h.module.pin_png(&pinned.pin.unwrap().pin_id).is_err());
    assert!(h.kernel.trigger_action(ACTION_CAPTURE).is_err());
    // 再次停用/退出可重复调用。
    h.module.on_shutdown(h.kernel.context()).unwrap();

    h.kernel.set_module_enabled("screenshot", true).unwrap();
    h.kernel.trigger_action(ACTION_CAPTURE).unwrap();
    assert!(h.module.has_active_session());
    h.kernel.shutdown().unwrap();
    assert!(!h.module.has_active_session());
}

/// 占用 F1 的测试模块，用于验证快捷键冲突时的回滚。
struct F1Holder;
impl AppModule for F1Holder {
    fn id(&self) -> &'static str {
        "f1_holder"
    }
    fn name(&self) -> &'static str {
        "F1 Holder"
    }
    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![ShortcutSpec::new(
            "f1_holder",
            "f1_holder:run",
            "F1",
            "occupy",
        )]
    }
}

#[test]
fn shortcut_conflict_on_enable_rolls_back_and_reports_error() {
    let dir = tempfile::tempdir().unwrap();
    let log: EventLog = Arc::new(Mutex::new(Vec::new()));
    let screen = Arc::new(FakeScreen::new(log.clone()));
    let module = Arc::new(ScreenshotModule::new(
        screen.clone(),
        screen.clone(),
        Arc::new(RecordingClipboard::default()),
    ));
    let backend = Arc::new(RecordingBackend {
        log,
        fail_create_prefix: Mutex::new(None),
    });
    let kernel = boot(
        dir.path(),
        backend,
        module.clone(),
        Some(Arc::new(F1Holder)),
    );
    // 启动时冲突被跳过：截图快捷键未注册，状态如实显示。
    assert!(kernel
        .context()
        .shortcut_registry
        .get_by_action(ACTION_CAPTURE)
        .is_none());
    assert!(!module.status(kernel.context()).shortcut.registered);

    kernel.set_module_enabled("screenshot", false).unwrap();
    let err = kernel.set_module_enabled("screenshot", true).unwrap_err();
    assert!(matches!(err, CoreError::ShortcutConflict { .. }), "{err}");
    assert!(!kernel.registry().is_enabled("screenshot"));
    assert_eq!(
        kernel
            .context()
            .shortcut_registry
            .get_by_action("f1_holder:run")
            .unwrap()
            .physical_accelerator,
        "F1"
    );

    // 改用自定义快捷键后可以启用。
    kernel
        .update_config(|c| {
            c.shortcut_overrides
                .insert(ACTION_CAPTURE.into(), "PrimaryMod+Shift+A".into());
        })
        .unwrap();
    kernel.set_module_enabled("screenshot", true).unwrap();
    assert_eq!(
        kernel
            .context()
            .shortcut_registry
            .get_by_action(ACTION_CAPTURE)
            .unwrap()
            .physical_accelerator,
        "Shift+Super+A"
    );
}

#[test]
fn settings_and_switch_survive_restart_with_unknown_fields_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let make = |path: &std::path::Path| {
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));
        let screen = Arc::new(FakeScreen::new(log.clone()));
        let module = Arc::new(ScreenshotModule::new(
            screen.clone(),
            screen.clone(),
            Arc::new(RecordingClipboard::default()),
        ));
        let backend = Arc::new(RecordingBackend {
            log,
            fail_create_prefix: Mutex::new(None),
        });
        let kernel = AppKernelBuilder::new(path)
            .with_target_os(TargetOs::MacOs)
            .with_window_backend(backend)
            .register_module_arc(module.clone())
            .unwrap()
            .bootstrap()
            .unwrap();
        (kernel, module)
    };
    let (kernel, module) = make(dir.path());
    let current = module.settings(kernel.context());
    let mut stored = serde_json::to_value(&current).unwrap();
    stored["future_flag"] = json!(42);
    stored["copy_after_save"] = json!(true);
    kernel
        .update_config(|c| {
            c.modules.settings.insert("screenshot".into(), stored);
            c.screenshot.show_magnifier = false;
        })
        .unwrap();
    kernel.set_module_enabled("screenshot", false).unwrap();
    kernel.shutdown().unwrap();
    drop(kernel);

    let (kernel, module) = make(dir.path());
    assert!(!kernel.registry().is_enabled("screenshot"));
    let settings = module.settings(kernel.context());
    assert!(settings.copy_after_save);
    assert_eq!(settings.unknown_fields["future_flag"], 42);
    assert!(!kernel.context().config.read().screenshot.show_magnifier);
    kernel.set_module_enabled("screenshot", true).unwrap();
    let info = module.start_capture(kernel.context(), None, false).unwrap();
    assert!(!info.settings.magnifier);
}

#[test]
fn copy_color_accepts_only_color_text() {
    let h = harness();
    h.module.copy_color("#A1B2C3").unwrap();
    h.module.copy_color("rgb(1, 2, 3)").unwrap();
    assert!(h.module.copy_color("rm -rf /").is_err());
    assert_eq!(
        h.clipboard.texts.lock().as_slice(),
        &["#A1B2C3".to_string(), "rgb(1, 2, 3)".to_string()]
    );
}

/// 测试用 OCR：返回相对输入图像的整图一行，并记录收到的尺寸。
struct RecordingOcr {
    seen: Mutex<Vec<(u32, u32)>>,
}

impl os_hal::OcrProvider for RecordingOcr {
    fn recognize_rgba(&self, frame: &CapturedFrame) -> HalResult<os_hal::OcrRecognitionResult> {
        self.seen.lock().push((frame.width, frame.height));
        Ok(os_hal::OcrRecognitionResult {
            backend_used: os_hal::OcrBackendKind::SystemNative,
            full_text: "fictional text".into(),
            lines: vec![os_hal::OcrLineResult {
                text: "fictional text".into(),
                confidence: 0.9,
                x: 0,
                y: 0,
                width: frame.width,
                height: frame.height,
            }],
            elapsed_ms: 1,
        })
    }

    fn release_idle_sessions(&self) -> HalResult<()> {
        Ok(())
    }
}

fn harness_with_ocr(ocr: Option<Arc<RecordingOcr>>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let log: EventLog = Arc::new(Mutex::new(Vec::new()));
    let screen = Arc::new(FakeScreen::new(log.clone()));
    let clipboard = Arc::new(RecordingClipboard::default());
    let backend = Arc::new(RecordingBackend {
        log: log.clone(),
        fail_create_prefix: Mutex::new(None),
    });
    let mut module = ScreenshotModule::new(screen.clone(), screen.clone(), clipboard.clone());
    if let Some(ocr) = ocr {
        module = module.with_ocr(ocr);
    }
    let module = Arc::new(module);
    let kernel = boot(&data_dir, backend.clone(), module.clone(), None);
    Harness {
        _dir: dir,
        data_dir,
        log,
        screen,
        clipboard,
        backend,
        module,
        kernel,
    }
}

#[test]
fn ui_hierarchy_is_reported_honestly_and_used_only_when_trusted() {
    let h = harness();
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    let caps = serde_json::to_value(&info.capabilities).unwrap();
    assert_eq!(caps["ui_elements"]["state"], "unavailable");
    assert!(caps["ui_elements"]["reason"]
        .as_str()
        .unwrap()
        .contains("辅助功能"));
    // 未授权：命中窗口，且不调用 AX。
    let hit = h
        .module
        .resolve_region(&info.session_id, 20, 15, 0)
        .unwrap()
        .unwrap();
    assert_eq!(hit.level, RegionHierarchyLevel::TopLevelWindow);
    assert!(!events(&h).iter().any(|e| e.starts_with("ax:")));

    h.screen.ui_trusted.store(true, Ordering::SeqCst);
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&info.capabilities).unwrap()["ui_elements"]["state"],
        "available"
    );
    // 帧像素 (20,15) → 桌面 (10,7)；按钮 (8,8,6x4) 点 → (16,16,12x8) 像素。
    let leaf = h
        .module
        .resolve_region(&info.session_id, 20, 18, 0)
        .unwrap()
        .unwrap();
    assert!(
        events(&h).iter().any(|e| e == "ax:10,9"),
        "{:?}",
        events(&h)
    );
    assert_eq!(leaf.level, RegionHierarchyLevel::UiElement);
    assert_eq!((leaf.x, leaf.y, leaf.width, leaf.height), (16, 16, 12, 8));
    let up = h
        .module
        .resolve_region(&info.session_id, 20, 18, 1)
        .unwrap()
        .unwrap();
    assert_eq!(up.level, RegionHierarchyLevel::TopLevelWindow);
}

#[test]
fn ocr_recognizes_cropped_original_frame_without_ending_session() {
    let ocr = Arc::new(RecordingOcr {
        seen: Mutex::new(Vec::new()),
    });
    let h = harness_with_ocr(Some(ocr.clone()));
    let info = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    let out = h
        .module
        .recognize_text(
            &info.session_id,
            SelectionBox {
                x: 70,
                y: 50,
                width: 30,
                height: 30,
            },
        )
        .unwrap();
    // 超出帧的部分被裁掉后才送 OCR。
    assert_eq!(ocr.seen.lock().as_slice(), &[(10, 10)]);
    assert_eq!((out.width, out.height), (10, 10));
    assert_eq!(out.text, "fictional text");
    assert!(h.module.has_active_session());
    h.module.copy_text(&out.text).unwrap();
    assert_eq!(h.clipboard.texts.lock().last().unwrap(), "fictional text");
    assert!(h.module.copy_text("   ").is_err());
}

#[test]
fn ocr_without_provider_reports_unsupported_and_shortcut_sets_intent() {
    let h = harness();
    h.kernel.trigger_action(mod_screenshot::ACTION_OCR).unwrap();
    let info = h.module.session_info(h.kernel.context(), None).unwrap();
    assert_eq!(info.intent, mod_screenshot::ocr::CaptureIntent::Ocr);
    let err = h
        .module
        .recognize_text(
            &info.session_id,
            SelectionBox {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            },
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("native_ocr"), "{err}");
    // 普通截图快捷键恢复为 Capture 意图。
    h.kernel.trigger_action(ACTION_CAPTURE).unwrap();
    assert_eq!(
        h.module
            .session_info(h.kernel.context(), None)
            .unwrap()
            .intent,
        mod_screenshot::ocr::CaptureIntent::Capture
    );
}

/// 状态轮询不得申请权限；曾经成功后撤销必须再次拒绝，不能复用永久成功缓存。
#[test]
fn permission_polling_is_passive_and_revocation_blocks_next_capture() {
    let h = harness();
    assert!(h.module.status(h.kernel.context()).permission.granted);
    assert_eq!(h.screen.permission_requests.load(Ordering::SeqCst), 0);
    let session = h
        .module
        .start_capture(h.kernel.context(), None, false)
        .unwrap();
    assert_eq!(h.screen.permission_requests.load(Ordering::SeqCst), 1);
    h.module
        .cancel_capture(h.kernel.context(), Some(&session.session_id))
        .unwrap();
    h.screen.deny_permission.store(true, Ordering::SeqCst);
    assert!(!h.module.status(h.kernel.context()).permission.granted);
    assert_eq!(h.screen.permission_requests.load(Ordering::SeqCst), 1);
    assert!(h
        .module
        .start_capture(h.kernel.context(), None, false)
        .is_err());
    assert_eq!(h.screen.captures.load(Ordering::SeqCst), 1);
    assert_eq!(h.screen.permission_requests.load(Ordering::SeqCst), 2);
    assert!(!h.module.has_active_session());
}
