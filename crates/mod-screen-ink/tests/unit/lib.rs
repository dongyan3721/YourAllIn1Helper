use super::*;
use app_core::{AppKernelBuilder, TargetOs};
use image::{ImageBuffer, Rgba};
use os_hal::MockOsHal;
use std::io::Cursor;

fn generate_test_ink_png(w: u32, h: u32, stroke_color: Rgba<u8>) -> Vec<u8> {
    let mut img = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(w, h);
    // 绘制一条横线作为透明墨迹
    for x in 0..w {
        img.put_pixel(x, h / 2, stroke_color);
    }
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

#[test]
fn test_screen_ink_initial_state_and_shortcuts() {
    let mock = MockOsHal::new();
    let module = ScreenInkModule::new(Arc::new(mock));

    assert_eq!(module.id(), "screen_ink");
    assert_eq!(module.name(), "桌面屏幕注释");

    let shortcuts = module.default_shortcuts();
    assert_eq!(shortcuts.len(), 2);
    assert_eq!(shortcuts[0].action_id, "screen_ink:toggle_session");
    assert_eq!(shortcuts[0].default_logical_hotkey, "PrimaryMod+Shift+D");
    assert_eq!(shortcuts[1].action_id, "screen_ink:toggle_passthrough");
    assert_eq!(shortcuts[1].default_logical_hotkey, "PrimaryMod+Shift+E");

    let status = module.get_status();
    assert!(!status.is_active);
    assert!(!status.is_passthrough);
}

#[test]
fn test_composite_screen_with_ink_exact_and_resized() {
    // 1. 生成 100x100 纯白底图 (255, 255, 255, 255)
    let bg_w = 100u32;
    let bg_h = 100u32;
    let bg_rgba = vec![255u8; (bg_w * bg_h * 4) as usize];

    // 2. 生成 100x100 半透明/纯红墨迹 (255, 0, 0, 255)
    let red_pixel = Rgba([255, 0, 0, 255]);
    let ink_png = generate_test_ink_png(bg_w, bg_h, red_pixel);

    // 3. 执行合成
    let (png_bytes, out_w, out_h) =
        composite::composite_screen_with_ink(bg_w, bg_h, &bg_rgba, &ink_png).unwrap();
    assert_eq!(out_w, 100);
    assert_eq!(out_h, 100);
    assert!(!png_bytes.is_empty());

    // 解码合成图验证叠加效果
    let decoded = image::load_from_memory(&png_bytes).unwrap().to_rgba8();
    let middle_pixel = decoded.get_pixel(50, 50);
    // 中间横线位置应该是红色
    assert_eq!(middle_pixel[0], 255);
    assert_eq!(middle_pixel[1], 0);
    assert_eq!(middle_pixel[2], 0);
    assert_eq!(middle_pixel[3], 255);

    // 4. 测试 HiDPI 尺寸不一致自动等比缩放合成（例如 50x50 墨迹叠加在 100x100 底图）
    let ink_small_png = generate_test_ink_png(50, 50, red_pixel);
    let (resized_png, r_w, r_h) =
        composite::composite_screen_with_ink(bg_w, bg_h, &bg_rgba, &ink_small_png).unwrap();
    assert_eq!(r_w, 100);
    assert_eq!(r_h, 100);
    assert!(!resized_png.is_empty());
}

#[test]
fn test_screen_ink_dual_window_session_and_passthrough_lifecycle() {
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

    // 1. 启动前：0 个 WebView
    assert_eq!(ctx.window_pool.active_window_count(), 0);
    assert!(!ink_module.get_status().is_active);

    // 2. 启动屏幕注释会话 -> 打开 canvas 与 toolbar 两个 WebView 窗口
    let snap = ink_module.start_session(ctx).unwrap();
    assert!(snap.is_active);
    assert!(!snap.is_passthrough);
    assert_eq!(ctx.window_pool.active_window_count(), 2);
    assert!(ctx
        .window_pool
        .get_active_window("screen-ink-canvas")
        .is_some());
    assert!(ctx
        .window_pool
        .get_active_window("screen-ink-toolbar")
        .is_some());

    // 3. 切换为操作桌面模式 (passthrough = true)
    // 关键校验：全屏画布设置为忽略鼠标事件，但独立工具栏不穿透
    let snap_pass = ink_module.set_passthrough(ctx, true).unwrap();
    assert!(snap_pass.is_passthrough);
    let canvas_win = ctx
        .window_pool
        .get_active_window("screen-ink-canvas")
        .unwrap();
    assert!(canvas_win.ignore_cursor_events);

    let toolbar_win = ctx
        .window_pool
        .get_active_window("screen-ink-toolbar")
        .unwrap();
    assert!(!toolbar_win.ignore_cursor_events); // 工具栏绝不穿透！

    // 4. 切回绘图模式 (passthrough = false)
    let snap_draw = ink_module.set_passthrough(ctx, false).unwrap();
    assert!(!snap_draw.is_passthrough);
    let canvas_win2 = ctx
        .window_pool
        .get_active_window("screen-ink-canvas")
        .unwrap();
    assert!(!canvas_win2.ignore_cursor_events);

    // 5. 退出会话 -> 立即销毁两个窗口，恢复 0 个 WebView
    ink_module.exit_session(ctx).unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 0);
    assert!(!ink_module.get_status().is_active);

    kernel.shutdown().unwrap();
}

#[test]
fn test_screen_ink_repeated_start_resets_passthrough() {
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

    // 首次启动并置为穿透
    ink_module.start_session(ctx).unwrap();
    ink_module.set_passthrough(ctx, true).unwrap();
    assert!(
        ctx.window_pool
            .get_active_window("screen-ink-canvas")
            .unwrap()
            .ignore_cursor_events
    );

    // 重复启动：状态被重置为非穿透，原生画布也必须立即重置为非穿透（不忽略鼠标）
    let snap = ink_module.start_session(ctx).unwrap();
    assert!(!snap.is_passthrough);
    assert!(
        !ctx.window_pool
            .get_active_window("screen-ink-canvas")
            .unwrap()
            .ignore_cursor_events
    );

    ink_module.exit_session(ctx).unwrap();
    kernel.shutdown().unwrap();
}

#[test]
fn test_screen_ink_composite_and_save_snapshot() {
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
    let ink_png = generate_test_ink_png(64, 64, Rgba([0, 255, 0, 255]));

    let target_file = temp_dir.path().join("snapshots").join("custom_snap.png");
    let result = ink_module
        .composite_and_save(
            ctx,
            &ink_png,
            false, // 单元测试环境禁用复制到物理剪贴板以免受环境约束
            Some(target_file.to_string_lossy().to_string()),
        )
        .unwrap();

    // MockOsHal 提供 64x64 单帧底图
    assert_eq!(result.width, 64);
    assert_eq!(result.height, 64);
    assert!(target_file.exists());
    assert!(std::fs::metadata(&target_file).unwrap().len() > 0);

    kernel.shutdown().unwrap();
}

#[test]
fn test_screen_ink_save_snapshot_exclusive_naming_no_overwrite() {
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
    let ink_png = generate_test_ink_png(64, 64, Rgba([0, 255, 0, 255]));

    // 配置保存目录到 temp_dir
    let save_dir = temp_dir.path().join("auto_snapshots");
    let mut cfg = ScreenInkConfig::default();
    cfg.save_directory = Some(save_dir.to_string_lossy().to_string());
    ink_module.apply_config(cfg);

    // 连续保存两次（同秒场景）
    let res1 = ink_module
        .composite_and_save(ctx, &ink_png, false, None)
        .unwrap();
    let res2 = ink_module
        .composite_and_save(ctx, &ink_png, false, None)
        .unwrap();

    let path1 = res1.saved_path.unwrap();
    let path2 = res2.saved_path.unwrap();

    // 验证两次保存生成了互不相同的独立文件，无覆盖
    assert_ne!(path1, path2);
    assert!(std::path::Path::new(&path1).exists());
    assert!(std::path::Path::new(&path2).exists());

    kernel.shutdown().unwrap();
}

#[test]
fn test_screen_ink_config_serde_defaults_and_sanitization() {
    // 1. 空 JSON 得到合法的默认值
    let empty_json = "{}";
    let cfg: ScreenInkConfig = serde_json::from_str(empty_json).unwrap();
    assert_eq!(cfg.default_tool, "pen");
    assert_eq!(cfg.default_color, "#ef4444");
    assert_eq!(cfg.default_width, 4.0);
    assert_eq!(cfg.save_directory, None);

    // 2. 局部 JSON 覆盖与缺省补齐
    let partial_json = r##"{"default_color": "#00FF00"}"##;
    let cfg2: ScreenInkConfig = serde_json::from_str(partial_json).unwrap();
    assert_eq!(cfg2.default_tool, "pen");
    assert_eq!(cfg2.default_color, "#00FF00");
    assert_eq!(cfg2.default_width, 4.0);

    // 3. 校验机制清洗非法值
    let invalid_json = r##"{
        "default_tool": "illegal_tool_name",
        "default_color": "not_a_hex_color",
        "default_width": 999.0,
        "save_directory": "   "
    }"##;
    let mut cfg3: ScreenInkConfig = serde_json::from_str(invalid_json).unwrap();
    cfg3.sanitize();
    assert_eq!(cfg3.default_tool, "pen");
    assert_eq!(cfg3.default_color, "#ef4444");
    assert_eq!(cfg3.default_width, 4.0); // 极端值重置为 4.0
    assert_eq!(cfg3.save_directory, None); // 空白字符串重置为 None
}

#[test]
fn test_screen_ink_config_applied_on_init() {
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

    // 注入自定义配置并调用 on_init
    let custom_cfg = serde_json::json!({
        "default_tool": "rect",
        "default_color": "#34C759",
        "default_width": 7.5
    });
    ctx.config
        .write()
        .modules
        .settings
        .insert("screen_ink".to_string(), custom_cfg);

    ink_module.on_init(ctx).unwrap();

    let status = ink_module.get_status();
    assert_eq!(status.active_tool, "rect");
    assert_eq!(status.color, "#34C759");
    assert_eq!(status.stroke_width, 7.5);

    kernel.shutdown().unwrap();
}

// 隐藏第二个窗口失败时必须恢复第一个，不能继续截取包含工具栏的脏底图。
#[test]
fn clean_capture_visibility_failure_restores_windows_and_can_retry() {
    use app_core::{WindowBackend, WindowSpec};
    use std::sync::atomic::{AtomicBool, Ordering};
    struct VisibilityBackend {
        fail_hide: AtomicBool,
        fail_restore: AtomicBool,
        events: parking_lot::Mutex<Vec<(String, bool)>>,
    }
    impl WindowBackend for VisibilityBackend {
        fn create_or_focus_window(&self, _: &WindowSpec) -> CoreResult<()> {
            Ok(())
        }
        fn destroy_window(&self, _: &str) -> CoreResult<()> {
            Ok(())
        }
        fn set_ignore_cursor_events(&self, _: &str, _: bool) -> CoreResult<()> {
            Ok(())
        }
        fn set_always_on_top(&self, _: &str, _: bool) -> CoreResult<()> {
            Ok(())
        }
        fn set_window_visible(&self, label: &str, visible: bool) -> CoreResult<()> {
            self.events.lock().push((label.into(), visible));
            if (!visible && label == "screen-ink-toolbar" && self.fail_hide.load(Ordering::SeqCst))
                || (visible && self.fail_restore.load(Ordering::SeqCst))
            {
                return Err(CoreError::Internal(anyhow!("visibility failure")));
            }
            Ok(())
        }
    }
    let backend = Arc::new(VisibilityBackend {
        fail_hide: AtomicBool::new(true),
        fail_restore: AtomicBool::new(false),
        events: Default::default(),
    });
    let pool = WindowPoolManager::new(backend.clone());
    let (canvas, toolbar) = WindowSpec::screen_ink_pair(800.0, 600.0);
    pool.open_window(canvas).unwrap();
    pool.open_window(toolbar).unwrap();
    assert!(
        CleanCaptureVisibilityGuard::new(&pool, &["screen-ink-canvas", "screen-ink-toolbar"])
            .is_err()
    );
    assert_eq!(
        *backend.events.lock(),
        vec![
            ("screen-ink-canvas".into(), false),
            ("screen-ink-toolbar".into(), false),
            ("screen-ink-canvas".into(), true)
        ]
    );
    backend.fail_hide.store(false, Ordering::SeqCst);
    let mut guard =
        CleanCaptureVisibilityGuard::new(&pool, &["screen-ink-canvas", "screen-ink-toolbar"])
            .unwrap();
    backend.fail_restore.store(true, Ordering::SeqCst);
    assert!(guard.restore().is_err());
    backend.fail_restore.store(false, Ordering::SeqCst);
    guard.restore().unwrap();
    assert!(guard.labels.is_empty());
}
