use super::*;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

struct MockScreenshotModule {
    capture_triggers: Arc<AtomicUsize>,
}

impl AppModule for MockScreenshotModule {
    fn id(&self) -> &'static str {
        "screenshot"
    }

    fn name(&self) -> &'static str {
        "增强截图与贴图"
    }

    fn migrations(&self) -> Vec<ModuleMigration> {
        vec![ModuleMigration::new(
            "screenshot",
            1,
            "Create ocr_models_meta table",
            "CREATE TABLE IF NOT EXISTS ocr_models_meta (
                    id TEXT PRIMARY KEY,
                    model_name TEXT NOT NULL,
                    det_path TEXT NOT NULL,
                    rec_path TEXT NOT NULL
                );",
        )]
    }

    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![
            ShortcutSpec::new(
                "screenshot",
                "screenshot:capture",
                "F1",
                "启动区域截图(支持手动框选与智能框推导)",
            ),
            ShortcutSpec::new(
                "screenshot",
                "screenshot:ocr",
                "PrimaryMod+Shift+O",
                "截图并立即 OCR",
            ),
        ]
    }

    fn on_shortcut(&self, action_id: &str, ctx: &ModuleContext) -> CoreResult<()> {
        if action_id == "screenshot:capture" {
            self.capture_triggers.fetch_add(1, Ordering::SeqCst);
            let enable_smart = ctx.config.read().screenshot.enable_smart_region_detection;
            ctx.event_bus.publish(SystemEvent::ScreenCaptureRequested {
                mode: CaptureMode::Standard,
                enable_smart_region_detection: enable_smart,
            });
            ctx.window_pool
                .open_window(WindowSpec::capture_overlay(0.0, 0.0, 1920.0, 1080.0))?;
        }
        Ok(())
    }
}

#[test]
fn test_snipaste_smart_region_detection_and_wheel_hierarchy() {
    let regions = vec![
        DetectedRegionRect {
            region_id: 1,
            parent_id: None,
            level: RegionHierarchyLevel::Monitor,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            label: Some("Primary Monitor".to_string()),
        },
        DetectedRegionRect {
            region_id: 2,
            parent_id: Some(1),
            level: RegionHierarchyLevel::TopLevelWindow,
            x: 100,
            y: 100,
            width: 800,
            height: 600,
            label: Some("VS Code".to_string()),
        },
        DetectedRegionRect {
            region_id: 3,
            parent_id: Some(2),
            level: RegionHierarchyLevel::ContainerPanel,
            x: 150,
            y: 150,
            width: 400,
            height: 300,
            label: Some("Editor Group".to_string()),
        },
        DetectedRegionRect {
            region_id: 4,
            parent_id: Some(3),
            level: RegionHierarchyLevel::UiElement,
            x: 180,
            y: 180,
            width: 120,
            height: 36,
            label: Some("Run Button".to_string()),
        },
    ];

    // Hover at (200, 190): should hit innermost UiElement ("Run Button", id=4)
    let hit0 = resolve_smart_region_at_point(&regions, 200, 190, 0).unwrap();
    assert_eq!(hit0.region_id, 4);

    // Scroll mouse wheel up 1 step: expands to ContainerPanel ("Editor Group", id=3)
    let hit1 = resolve_smart_region_at_point(&regions, 200, 190, 1).unwrap();
    assert_eq!(hit1.region_id, 3);

    // Scroll mouse wheel up 2 steps: expands to TopLevelWindow ("VS Code", id=2)
    let hit2 = resolve_smart_region_at_point(&regions, 200, 190, 2).unwrap();
    assert_eq!(hit2.region_id, 2);

    // Scroll mouse wheel up 3 steps: expands to Monitor (id=1)
    let hit3 = resolve_smart_region_at_point(&regions, 200, 190, 3).unwrap();
    assert_eq!(hit3.region_id, 1);
}

#[test]
fn test_cross_platform_shortcut_normalization() {
    let win_accel =
        ShortcutNormalizer::normalize_for_os("PrimaryMod+Shift+V", TargetOs::Windows).unwrap();
    assert_eq!(win_accel, "Ctrl+Shift+V");

    let mac_accel =
        ShortcutNormalizer::normalize_for_os("PrimaryMod+Shift+V", TargetOs::MacOs).unwrap();
    assert_eq!(mac_accel, "Shift+Super+V");

    let mac_badge =
        ShortcutNormalizer::format_display_label("PrimaryMod+Shift+V", TargetOs::MacOs).unwrap();
    assert_eq!(mac_badge, "⇧ + ⌘ + V");
}

#[test]
fn test_config_migration_v1_to_v2_preserves_unknown_fields() {
    let legacy_v1 = json!({
        "config_version": 1,
        "max_clipboard_items": 150,
        "future_plugin_flag": "keep_me"
    });

    let migrated = ConfigMigrationPipeline::migrate_value(legacy_v1, 1).unwrap();
    let cfg: AppConfig = serde_json::from_value(migrated).unwrap();

    assert_eq!(cfg.config_version, CURRENT_CONFIG_VERSION);
    assert_eq!(cfg.clipboard.max_history_items, 150);
    assert!(cfg.screenshot.enable_manual_box_selection);
    assert!(cfg.screenshot.enable_smart_region_detection);
    assert_eq!(
        cfg.unknown_fields.get("future_plugin_flag"),
        Some(&Value::String("keep_me".to_string()))
    );
}

#[test]
fn test_kernel_bootstrap_zero_webview_and_shortcut_dispatch() {
    let temp_dir = tempfile::tempdir().unwrap();
    let triggers = Arc::new(AtomicUsize::new(0));

    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_target_os(TargetOs::MacOs)
        .register_module(MockScreenshotModule {
            capture_triggers: Arc::clone(&triggers),
        })
        .unwrap()
        .bootstrap()
        .unwrap();

    // Level 0 guarantee: 0 WebView windows open after daemon startup
    assert_eq!(kernel.context().window_pool.active_window_count(), 0);

    // Trigger F1 screenshot shortcut
    kernel.trigger_accelerator("F1").unwrap();
    assert_eq!(triggers.load(Ordering::SeqCst), 1);
    assert_eq!(kernel.context().window_pool.active_window_count(), 1);

    // Destroy transient overlay after capture completes -> back to 0 WebViews
    kernel
        .context()
        .window_pool
        .destroy_window("capture-overlay")
        .unwrap();
    assert_eq!(kernel.context().window_pool.active_window_count(), 0);

    kernel.shutdown().unwrap();
}

#[test]
fn visual_edge_box_handles_full_white_pixels_without_overflow() {
    // 白色卡片位于黑底中央：旧实现以 i16 计算亮度，255×150 溢出导致调试构建 panic。
    let (w, h) = (64u32, 48u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 10..30 {
        for x in 20..50 {
            let i = ((y * w + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    let found = crate::infer_visual_edge_box_rgba(&rgba, w, h, 30, 20, 28).unwrap();
    assert_eq!(
        (found.x, found.y, found.width, found.height),
        (20, 10, 30, 20)
    );
}
