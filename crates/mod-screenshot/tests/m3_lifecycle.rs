//! 功能 01 截图 M3 生命周期集成测试。
//!
//! 验证：
//! 1. 滚动长截图全流程：开启 → 采集拼接 → 停止预览 → 复用 M1 保存/复制/贴图；
//! 2. GIF 录制全流程：开启 → 定时采集与流式编码 → 停止预览与文件生成；
//! 3. 模块停用/关闭时优雅取消并等待所有后台线程结束，销毁自有窗口。

use app_core::{
    AppKernelBuilder, AppModule, CoreResult, SelectionBox, TargetOs, WindowBackend, WindowSpec,
};
use mod_screenshot::{
    export::ExportAction,
    gif::{GifFramerate, GifRecordingConfig, GifResolutionPreset},
    ScreenshotModule, CONTROL_LABEL, OVERLAY_LABEL, PREVIEW_LABEL,
};
use os_hal::{CapturedFrame, DisplayMonitorInfo, HalResult, ScreenCapturer};
use std::{sync::Arc, time::Duration};
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

#[test]
fn test_scroll_full_lifecycle_and_export_reusing_m1() {
    let dir = tempdir().unwrap();
    let capturer = Arc::new(TestCapturer {
        step: parking_lot::Mutex::new(0),
    });
    let detector = Arc::new(os_hal::MockOsHal::new());
    let clipboard = Arc::new(MemoryClipboard {
        has_image: parking_lot::Mutex::new(false),
    });

    let module = Arc::new(ScreenshotModule::new(capturer, detector, clipboard.clone()));
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .with_target_os(TargetOs::MacOs)
        .with_window_backend(Arc::new(DummyBackend))
        .register_module_arc(module.clone())
        .expect("Kernel builder")
        .bootstrap()
        .expect("Kernel bootstrap");
    let ctx = kernel.context();

    // 1. 开启截图会话
    let session_info = module
        .start_capture(ctx, None, false)
        .expect("start_capture");
    assert!(ctx.window_pool.get_active_window(OVERLAY_LABEL).is_some());

    // 2. 选区后开启长截图
    let sel = SelectionBox {
        x: 50,
        y: 50,
        width: 160,
        height: 120,
    };
    let scroll_status = module
        .start_scroll(ctx, &session_info.session_id, sel)
        .expect("start_scroll");
    assert!(scroll_status.is_active);

    // 遮罩已销毁，选区外控制条已创建
    assert!(ctx.window_pool.get_active_window(OVERLAY_LABEL).is_none());
    assert!(ctx.window_pool.get_active_window(CONTROL_LABEL).is_some());

    // 3. 运行约 200ms
    std::thread::sleep(Duration::from_millis(200));

    // 4. 停止长截图
    let stop_status = module.stop_scroll(ctx).expect("stop_scroll");
    assert!(!stop_status.is_active);

    // 控制条已销毁，预览窗口已开启
    assert!(ctx.window_pool.get_active_window(CONTROL_LABEL).is_none());
    assert!(ctx.window_pool.get_active_window(PREVIEW_LABEL).is_some());

    // 5. 复用 M1 导出：复制
    let outcome = module
        .export_scroll(ctx, ExportAction::Copy)
        .expect("export_scroll copy");
    assert!(outcome.copied);
    assert!(*clipboard.has_image.lock());
    // 导出完成后预览窗口关闭
    assert!(ctx.window_pool.get_active_window(PREVIEW_LABEL).is_none());
}

#[test]
fn test_gif_full_lifecycle_and_file_creation() {
    let dir = tempdir().unwrap();
    let capturer = Arc::new(TestCapturer {
        step: parking_lot::Mutex::new(0),
    });
    let detector = Arc::new(os_hal::MockOsHal::new());
    let clipboard = Arc::new(MemoryClipboard {
        has_image: parking_lot::Mutex::new(false),
    });

    let module = Arc::new(ScreenshotModule::new(capturer, detector, clipboard));
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .with_target_os(TargetOs::MacOs)
        .with_window_backend(Arc::new(DummyBackend))
        .register_module_arc(module.clone())
        .expect("Kernel builder")
        .bootstrap()
        .expect("Kernel bootstrap");
    let ctx = kernel.context();

    let session_info = module
        .start_capture(ctx, None, false)
        .expect("start_capture");

    let sel = SelectionBox {
        x: 20,
        y: 20,
        width: 100,
        height: 80,
    };
    let config = GifRecordingConfig {
        resolution_preset: GifResolutionPreset::Original,
        framerate: GifFramerate::Fps15,
        max_duration_secs: 10,
        max_frames: 100,
        sample_factor: 10,
    };

    let gif_status = module
        .start_gif(ctx, &session_info.session_id, sel, config)
        .expect("start_gif");
    assert!(gif_status.is_recording);

    assert!(ctx.window_pool.get_active_window(OVERLAY_LABEL).is_none());
    assert!(ctx.window_pool.get_active_window(CONTROL_LABEL).is_some());

    std::thread::sleep(Duration::from_millis(200));

    let stop_status = module.stop_gif(ctx).expect("stop_gif");
    assert!(!stop_status.is_recording);
    assert!(stop_status.saved_path.is_some());

    assert!(ctx.window_pool.get_active_window(CONTROL_LABEL).is_none());
    assert!(ctx.window_pool.get_active_window(PREVIEW_LABEL).is_some());

    let saved_file = std::path::PathBuf::from(stop_status.saved_path.unwrap());
    assert!(saved_file.exists());
    let file_data = std::fs::read(&saved_file).unwrap();
    let reader = gif::DecodeOptions::new()
        .read_info(std::io::Cursor::new(&file_data))
        .expect("Valid decodable GIF file");
    assert_eq!(reader.width(), 100);
    assert_eq!(reader.height(), 80);
}

#[test]
fn test_shutdown_cancels_and_waits_for_workers() {
    let dir = tempdir().unwrap();
    let capturer = Arc::new(TestCapturer {
        step: parking_lot::Mutex::new(0),
    });
    let detector = Arc::new(os_hal::MockOsHal::new());
    let clipboard = Arc::new(MemoryClipboard {
        has_image: parking_lot::Mutex::new(false),
    });

    let module = Arc::new(ScreenshotModule::new(capturer, detector, clipboard));
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .with_target_os(TargetOs::MacOs)
        .with_window_backend(Arc::new(DummyBackend))
        .register_module_arc(module.clone())
        .expect("Kernel builder")
        .bootstrap()
        .expect("Kernel bootstrap");
    let ctx = kernel.context();

    let session_info = module
        .start_capture(ctx, None, false)
        .expect("start_capture");

    let sel = SelectionBox {
        x: 10,
        y: 10,
        width: 80,
        height: 80,
    };
    module
        .start_scroll(ctx, &session_info.session_id, sel)
        .expect("start_scroll");
    assert!(module.scroll_status().unwrap().is_active);

    // 停用模块时触发 on_shutdown
    module.on_shutdown(ctx).expect("shutdown");

    // 验证会话已停止，所有窗口已清理
    assert!(module.scroll_status().is_none());
    assert!(ctx.window_pool.get_active_window(CONTROL_LABEL).is_none());
    assert!(ctx.window_pool.get_active_window(PREVIEW_LABEL).is_none());
}
