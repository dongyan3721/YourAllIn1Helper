//! 「另存为」完整路径保存与一键默认路径保存的集成测试。
//!
//! 使用显式注入的测试适配器（自制帧、内存剪贴板、记录型窗口后端），不读取真实屏幕、
//! 不写系统剪贴板；所有文件写入临时目录（含中文/空格路径）。
use app_core::{
    AppKernelBuilder, AppModule, CoreResult, SelectionBox, TargetOs, WindowBackend, WindowSpec,
};
use mod_screenshot::{
    export::ExportAction,
    gif::{GifFramerate, GifRecordingConfig, GifResolutionPreset},
    ScreenshotModule, OVERLAY_LABEL,
};
use os_hal::{CapturedFrame, DisplayMonitorInfo, HalResult, ScreenCapturer};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tempfile::tempdir;

/// 用于测试的动态彩色条纹测试屏幕。
struct TestCapturer {
    step: parking_lot::Mutex<u32>,
}

impl ScreenCapturer for TestCapturer {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        Ok(vec![DisplayMonitorInfo {
            id: 1,
            name: "Primary".to_string(),
            x: 0,
            y: 0,
            width: 400,
            height: 300,
            scale_factor: 1.0,
            is_primary: true,
        }])
    }

    fn capture_monitor(&self, _id: Option<u32>) -> HalResult<CapturedFrame> {
        Ok(CapturedFrame {
            width: 400,
            height: 300,
            scale_factor_milli: 1000,
            rgba_bytes: vec![120u8; 400 * 300 * 4],
        })
    }

    fn capture_region(&self, bounds: SelectionBox) -> HalResult<CapturedFrame> {
        let mut s = self.step.lock();
        *s += 1;
        let offset = *s * 20;
        let mut rgba = Vec::with_capacity((bounds.width * bounds.height * 4) as usize);
        for y in 0..bounds.height {
            let actual_y = y + offset;
            for x in 0..bounds.width {
                let r = ((x * 19 + actual_y * 11) % 251) as u8;
                let g = ((x * 7 + actual_y * 37) % 241) as u8;
                let b = ((x * 3 + actual_y * 17) % 253) as u8;
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Ok(CapturedFrame {
            width: bounds.width,
            height: bounds.height,
            scale_factor_milli: 1000,
            rgba_bytes: rgba,
        })
    }
}

/// 模拟内存剪贴板。
struct MemoryClipboard {
    has_image: parking_lot::Mutex<bool>,
}

impl mod_screenshot::clipboard::ScreenshotClipboard for MemoryClipboard {
    fn write_image(&self, _w: u32, _h: u32, _rgba: &[u8]) -> Result<(), String> {
        *self.has_image.lock() = true;
        Ok(())
    }
    fn write_text(&self, _text: &str) -> Result<(), String> {
        Ok(())
    }
}

/// 简单窗口后端，仅在内存中维护窗口开启/关闭状态。
struct DummyBackend;

impl WindowBackend for DummyBackend {
    fn create_or_focus_window(&self, _spec: &WindowSpec) -> CoreResult<()> {
        Ok(())
    }
    fn destroy_window(&self, _label: &str) -> CoreResult<()> {
        Ok(())
    }
    fn set_ignore_cursor_events(&self, _label: &str, _ignore: bool) -> CoreResult<()> {
        Ok(())
    }
    fn set_always_on_top(&self, _label: &str, _a: bool) -> CoreResult<()> {
        Ok(())
    }
    fn set_window_visible(&self, _label: &str, _v: bool) -> CoreResult<()> {
        Ok(())
    }
}

fn build_module(dir: &std::path::Path) -> (Arc<ScreenshotModule>, app_core::AppKernel) {
    let capturer = Arc::new(TestCapturer {
        step: parking_lot::Mutex::new(0),
    });
    let detector = Arc::new(os_hal::MockOsHal::new());
    let clipboard = Arc::new(MemoryClipboard {
        has_image: parking_lot::Mutex::new(false),
    });
    let module = Arc::new(ScreenshotModule::new(capturer, detector, clipboard));
    let kernel = AppKernelBuilder::new(dir)
        .with_in_memory_db(true)
        .with_target_os(TargetOs::MacOs)
        .with_window_backend(Arc::new(DummyBackend))
        .register_module_arc(module.clone())
        .expect("Kernel builder")
        .bootstrap()
        .expect("Kernel bootstrap");
    (module, kernel)
}

fn gif_config() -> GifRecordingConfig {
    GifRecordingConfig {
        resolution_preset: GifResolutionPreset::Original,
        framerate: GifFramerate::Fps15,
        max_duration_secs: 10,
        max_frames: 100,
        sample_factor: 10,
    }
}

#[test]
fn region_export_save_as_writes_chosen_path_and_default_save_still_works() {
    let dir = tempdir().unwrap();
    let (module, kernel) = build_module(dir.path());
    let ctx = kernel.context();
    let sel = SelectionBox {
        x: 2,
        y: 3,
        width: 4,
        height: 2,
    };

    // 1. 「另存为」：完整路径（中文+空格、缺扩展名）→ 补 .png 并写入。
    let chosen = dir.path().join("另存 文件").join("区域截图");
    let info = module.start_capture(ctx, None, false).unwrap();
    let out = module
        .export(
            ctx,
            &info.session_id,
            sel,
            None,
            ExportAction::Save,
            Some(&chosen),
        )
        .unwrap();
    let saved = PathBuf::from(out.saved_path.unwrap());
    assert_eq!(saved, chosen.with_extension("png"));
    assert!(saved.exists());

    // 2. 同一路径再次「另存为」→ 覆盖写入（对话框已确认覆盖）。
    let info = module.start_capture(ctx, None, false).unwrap();
    let out = module
        .export(
            ctx,
            &info.session_id,
            sel,
            None,
            ExportAction::Save,
            Some(&chosen),
        )
        .unwrap();
    assert_eq!(PathBuf::from(out.saved_path.unwrap()), saved);

    // 3. 快捷默认保存：不传路径仍走防覆盖落盘，且不会写到「另存为」路径。
    let info = module.start_capture(ctx, None, false).unwrap();
    let out = module
        .export(ctx, &info.session_id, sel, None, ExportAction::Save, None)
        .unwrap();
    let default_path = PathBuf::from(out.saved_path.unwrap());
    assert_ne!(default_path, saved);
    assert!(default_path.exists());
}

#[test]
fn scroll_export_save_as_writes_chosen_chinese_path() {
    let dir = tempdir().unwrap();
    let (module, kernel) = build_module(dir.path());
    let ctx = kernel.context();

    let info = module.start_capture(ctx, None, false).unwrap();
    let sel = SelectionBox {
        x: 50,
        y: 50,
        width: 160,
        height: 120,
    };
    module
        .start_scroll(ctx, &info.session_id, sel)
        .expect("start_scroll");
    std::thread::sleep(Duration::from_millis(200));
    module.stop_scroll(ctx).expect("stop_scroll");

    let chosen = dir.path().join("长截图 目录").join("拼接 结果.gif.png");
    let outcome = module
        .export_scroll(ctx, ExportAction::Save, Some(&chosen))
        .expect("export_scroll save_as");
    // 显式扩展名保持原样（.gif.png 是用户明确书写的完整文件名）。
    assert_eq!(PathBuf::from(outcome.saved_path.unwrap()), chosen);
    assert!(chosen.exists());

    // 快捷默认保存仍可用且不覆盖「另存为」产物。
    let outcome = module
        .export_scroll(ctx, ExportAction::Save, None)
        .expect("export_scroll default save");
    let default_path = PathBuf::from(outcome.saved_path.unwrap());
    assert_ne!(default_path, chosen);
    assert!(default_path.exists());
}

#[test]
fn pin_save_as_writes_chosen_path() {
    let dir = tempdir().unwrap();
    let (module, kernel) = build_module(dir.path());
    let ctx = kernel.context();

    let info = module.start_capture(ctx, None, false).unwrap();
    let out = module
        .export(
            ctx,
            &info.session_id,
            SelectionBox {
                x: 10,
                y: 10,
                width: 8,
                height: 8,
            },
            None,
            ExportAction::Pin,
            None,
        )
        .unwrap();
    let pin = out.pin.unwrap();

    let chosen = dir.path().join("贴图 备份.png");
    let saved = module
        .save_pin(ctx, &pin.pin_id, Some(&chosen))
        .expect("save_pin save_as");
    assert_eq!(PathBuf::from(saved), chosen);
    assert!(chosen.exists());

    // 快捷默认保存仍可用。
    let saved = module
        .save_pin(ctx, &pin.pin_id, None)
        .expect("save_pin default");
    assert_ne!(PathBuf::from(saved), chosen);
}

#[test]
fn gif_confirm_save_as_writes_chosen_path_and_cleans_pending() {
    let dir = tempdir().unwrap();
    let (module, kernel) = build_module(dir.path());
    let ctx = kernel.context();

    let info = module.start_capture(ctx, None, false).unwrap();
    let sel = SelectionBox {
        x: 20,
        y: 20,
        width: 100,
        height: 80,
    };
    module
        .start_gif(ctx, &info.session_id, sel, gif_config(), None)
        .expect("start_gif");
    std::thread::sleep(Duration::from_millis(200));
    let stopped = module.stop_gif(ctx).expect("stop_gif");
    assert!(stopped.pending_path.is_some());

    let chosen = dir.path().join("动图 目录").join("录制结果");
    let confirmed = module
        .confirm_gif(ctx, Some(&chosen))
        .expect("confirm_gif save_as");
    let saved = PathBuf::from(confirmed.saved_path.unwrap());
    // 缺扩展名 → 补 .gif。
    assert_eq!(saved, chosen.with_extension("gif"));
    assert!(saved.exists());
    // 临时文件已清理，遮罩已结束。
    assert_eq!(confirmed.pending_path, None);
    assert!(ctx.window_pool.get_active_window(OVERLAY_LABEL).is_none());

    // 已确认文件仍可读回（预览/复制另存的数据源）。
    let bytes = module.gif_bytes().expect("gif_bytes after confirm");
    assert!(!bytes.is_empty());
    let reader = gif::DecodeOptions::new()
        .read_info(std::io::Cursor::new(&bytes))
        .expect("Valid decodable GIF file");
    assert_eq!(reader.width(), 100);
    assert_eq!(reader.height(), 80);
}

#[test]
fn gif_confirm_default_save_still_works_and_save_gif_copy_does_not_consume() {
    let dir = tempdir().unwrap();
    let (module, kernel) = build_module(dir.path());
    let ctx = kernel.context();

    let info = module.start_capture(ctx, None, false).unwrap();
    let sel = SelectionBox {
        x: 10,
        y: 10,
        width: 60,
        height: 40,
    };
    module
        .start_gif(ctx, &info.session_id, sel, gif_config(), None)
        .expect("start_gif");
    std::thread::sleep(Duration::from_millis(200));
    module.stop_gif(ctx).expect("stop_gif");

    // 复制另存：写到指定路径（缺扩展名补 .gif）但不改变待确认状态。
    let copy_target = dir.path().join("复制 一份");
    let copied = module.save_gif_copy(&copy_target).expect("save_gif_copy");
    assert_eq!(PathBuf::from(copied), copy_target.with_extension("gif"));
    assert!(copy_target.with_extension("gif").exists());

    // 之后仍可一键默认保存（防覆盖落盘到默认目录）。
    let confirmed = module.confirm_gif(ctx, None).expect("confirm_gif default");
    let saved = PathBuf::from(confirmed.saved_path.unwrap());
    assert!(saved.exists());
    assert_ne!(saved, copy_target.with_extension("gif"));
    // 复制另存的产物不受后续确认影响。
    assert!(copy_target.with_extension("gif").exists());
}
