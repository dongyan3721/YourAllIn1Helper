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

    let bg_frame = CapturedFrame {
        width: 120,
        height: 80,
        scale_factor_milli: 1000,
        rgba_bytes: vec![200u8; 120 * 80 * 4],
    };

    let ink_png = generate_test_ink_png(120, 80, Rgba([0, 255, 0, 255]));

    let target_file = temp_dir.path().join("snapshots").join("custom_snap.png");
    let result = ink_module
        .composite_and_save(
            ctx,
            &ink_png,
            false, // 单元测试环境禁用复制到物理剪贴板以免受环境约束
            Some(target_file.to_string_lossy().to_string()),
            Some(bg_frame),
        )
        .unwrap();

    assert_eq!(result.width, 120);
    assert_eq!(result.height, 80);
    assert!(target_file.exists());
    assert!(std::fs::metadata(&target_file).unwrap().len() > 0);

    kernel.shutdown().unwrap();
}
