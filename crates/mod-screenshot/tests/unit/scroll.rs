//! 滚动长截图单元测试。
//!
//! 验证：
//! 1. 合成长页面按已知位移切帧 → 拼接结果逐像素一致；
//! 2. 固定头尾导航栏识别与保持；
//! 3. 重复帧与无位移平滑过滤；
//! 4. 低置信度安全熔断并保留已拼部分；
//! 5. 长度与内存上限约束；
//! 6. 会话取消释放所有帧。

use crate::scroll::{
    ncc::{calculate_displacement, detect_fixed_footer, detect_fixed_header, NccConfig},
    session::ScrollCaptureSession,
    stitcher::{ScrollStitcher, StitchStepResult, StitcherConfig, StopReason},
};
use os_hal::{CapturedFrame, DisplayMonitorInfo, HalResult, ScreenCapturer};
use std::sync::Arc;

/// 生成带有非平坦高频特征的合成图像（宽度 W，高度 H）。
fn generate_synthetic_page(width: usize, height: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 17 + y * 31 + (x ^ y)) % 251) as u8;
            let g = ((x * 23 + y * 13 + (x * y)) % 241) as u8;
            let b = ((x * 7 + y * 47 + 53) % 256) as u8;
            let a = 255;
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    rgba
}

/// 从大图中裁剪出指定行范围的切片帧。
fn slice_frame(src: &[u8], width: usize, y_start: usize, height: usize) -> Vec<u8> {
    let start_byte = y_start * width * 4;
    let len = height * width * 4;
    src[start_byte..start_byte + len].to_vec()
}

#[test]
fn test_scroll_stitch_known_displacement_pixel_perfect() {
    let width = 120;
    let full_height = 600;
    let page = generate_synthetic_page(width, full_height);

    let view_height = 150;
    let mut stitcher = ScrollStitcher::new(StitcherConfig {
        max_height: 2000,
        ..Default::default()
    });

    // 模拟滚动切帧：
    // Frame 0: rows [0, 150)
    // Frame 1: rows [30, 180)  (dy = 30)
    // Frame 2: rows [75, 225)  (dy = 45)
    // Frame 3: rows [135, 285) (dy = 60)
    let f0 = slice_frame(&page, width, 0, view_height);
    let f1 = slice_frame(&page, width, 30, view_height);
    let f2 = slice_frame(&page, width, 75, view_height);
    let f3 = slice_frame(&page, width, 135, view_height);

    let r0 = stitcher.push_frame(&f0, width, view_height);
    assert!(matches!(r0, StitchStepResult::Stitched { dy: 0, .. }));

    let r1 = stitcher.push_frame(&f1, width, view_height);
    assert_eq!(
        r1,
        StitchStepResult::Stitched {
            dy: 30,
            current_height: 180,
            confidence: 1.0
        }
    );

    let r2 = stitcher.push_frame(&f2, width, view_height);
    assert_eq!(
        r2,
        StitchStepResult::Stitched {
            dy: 45,
            current_height: 225,
            confidence: 1.0
        }
    );

    let r3 = stitcher.push_frame(&f3, width, view_height);
    assert_eq!(
        r3,
        StitchStepResult::Stitched {
            dy: 60,
            current_height: 285,
            confidence: 1.0
        }
    );

    let final_image = stitcher.finish(None);
    assert_eq!(final_image.width, width as u32);
    assert_eq!(final_image.height, 285);
    assert_eq!(final_image.stop_reason, StopReason::UserStopped);

    // 关键断言：拼接产物与原始合成长页面前 285 行逐像素 100% 一致！
    let expected_slice = &page[0..(width * 285 * 4)];
    assert_eq!(final_image.rgba_bytes, expected_slice);
}

#[test]
fn test_scroll_fixed_header_and_footer_detection_and_retention() {
    let width = 100;
    let height = 200;
    let header_h = 25;
    let footer_h = 20;

    let base_page = generate_synthetic_page(width, 500);

    // 构造带固定红页眉（25px）和固定蓝页脚（20px）的帧
    let make_frame_with_fixed_bars = |scroll_offset: usize| -> Vec<u8> {
        let mut frame = slice_frame(&base_page, width, scroll_offset, height);
        // 覆盖顶部 25 行红色
        for y in 0..header_h {
            for x in 0..width {
                let idx = (y * width + x) * 4;
                frame[idx] = 255;
                frame[idx + 1] = 0;
                frame[idx + 2] = 0;
                frame[idx + 3] = 255;
            }
        }
        // 覆盖底部 20 行蓝色
        for y in (height - footer_h)..height {
            for x in 0..width {
                let idx = (y * width + x) * 4;
                frame[idx] = 0;
                frame[idx + 1] = 0;
                frame[idx + 2] = 255;
                frame[idx + 3] = 255;
            }
        }
        frame
    };

    let f0 = make_frame_with_fixed_bars(0);
    let f1 = make_frame_with_fixed_bars(40);

    let config = NccConfig::default();
    let detected_hdr = detect_fixed_header(&f0, &f1, width, height, &config);
    let detected_ftr = detect_fixed_footer(&f0, &f1, width, height, &config);

    assert_eq!(detected_hdr, header_h);
    assert_eq!(detected_ftr, footer_h);

    let match_res =
        calculate_displacement(&f0, &f1, width, height, detected_hdr, detected_ftr, &config);
    assert!(match_res.is_some());
    let m = match_res.unwrap();
    assert_eq!(m.dy, 40);
    assert!(m.confidence > 0.95);

    // 测试拼接器处理固定头尾完整链路
    let mut stitcher = ScrollStitcher::new(StitcherConfig::default());
    stitcher.push_frame(&f0, width, height);
    let r1 = stitcher.push_frame(&f1, width, height);
    assert!(matches!(r1, StitchStepResult::Stitched { dy: 40, .. }));

    let final_img = stitcher.finish(None);
    // 最终高度 = 初始高度 (200) + dy (40) = 240
    assert_eq!(final_img.height, 240);

    // 检查页眉依然是红色
    for x in 0..width {
        let idx = x * 4;
        assert_eq!(&final_img.rgba_bytes[idx..idx + 3], &[255, 0, 0]);
    }
    // 检查页脚依然是蓝色
    let last_row_start = (240 - 1) * width * 4;
    for x in 0..width {
        let idx = last_row_start + x * 4;
        assert_eq!(&final_img.rgba_bytes[idx..idx + 3], &[0, 0, 255]);
    }
}

#[test]
fn test_scroll_duplicate_frame_and_zero_displacement() {
    let width = 80;
    let height = 120;
    let page = generate_synthetic_page(width, 300);

    let f0 = slice_frame(&page, width, 0, height);
    let f1 = slice_frame(&page, width, 35, height);

    let mut stitcher = ScrollStitcher::new(StitcherConfig::default());
    stitcher.push_frame(&f0, width, height);
    assert_eq!(stitcher.current_height(), height);

    // 重复推入相同的 f0
    let dup_res = stitcher.push_frame(&f0, width, height);
    assert_eq!(dup_res, StitchStepResult::ZeroDisplacement);
    assert_eq!(stitcher.current_height(), height);

    // 再推入 f1，应正常继续拼接
    let next_res = stitcher.push_frame(&f1, width, height);
    assert!(matches!(
        next_res,
        StitchStepResult::Stitched { dy: 35, .. }
    ));
    assert_eq!(stitcher.current_height(), height + 35);
}

#[test]
fn test_scroll_low_confidence_stops_and_preserves_canvas() {
    let width = 80;
    let height = 120;
    let page = generate_synthetic_page(width, 300);

    let f0 = slice_frame(&page, width, 0, height);
    let f1 = slice_frame(&page, width, 25, height);

    // 构造完全无关的噪点帧
    let garbage_frame = vec![128u8; width * height * 4];

    let mut stitcher = ScrollStitcher::new(StitcherConfig {
        max_low_confidence_skips: 1,
        ..Default::default()
    });

    stitcher.push_frame(&f0, width, height);
    stitcher.push_frame(&f1, width, height);
    let valid_height = stitcher.current_height();
    assert_eq!(valid_height, height + 25);

    // 第一次推入垃圾帧：跳过
    let skip1 = stitcher.push_frame(&garbage_frame, width, height);
    assert_eq!(skip1, StitchStepResult::ZeroDisplacement);

    // 第二次连续推入垃圾帧：熔断停止！
    let stop_res = stitcher.push_frame(&garbage_frame, width, height);
    match stop_res {
        StitchStepResult::Stopped(StopReason::LowConfidence { .. }) => {}
        other => panic!("Expected Stopped(LowConfidence), got {:?}", other),
    }

    // 关键保证：已拼好的有效高度依然完整保留！绝不出错位图！
    assert_eq!(stitcher.current_height(), valid_height);
    let finished = stitcher.finish(None);
    assert_eq!(finished.height as usize, valid_height);
}

#[test]
fn test_scroll_length_and_memory_limit() {
    let width = 60;
    let height = 160;
    let page = generate_synthetic_page(width, 1000);

    // 设置最大允许高度 220 像素
    let mut stitcher = ScrollStitcher::new(StitcherConfig {
        max_height: 220,
        ..Default::default()
    });

    let f0 = slice_frame(&page, width, 0, height);
    let f1 = slice_frame(&page, width, 40, height); // 160 + 40 = 200
    let f2 = slice_frame(&page, width, 80, height); // 200 + 40 = 240 > 220

    stitcher.push_frame(&f0, width, height);
    let r1 = stitcher.push_frame(&f1, width, height);
    assert!(matches!(r1, StitchStepResult::Stitched { dy: 40, .. }));
    assert_eq!(stitcher.current_height(), 200);

    let over_res = stitcher.push_frame(&f2, width, height);
    assert_eq!(
        over_res,
        StitchStepResult::Stopped(StopReason::MaxLengthReached { max_height: 220 })
    );
    assert_eq!(stitcher.current_height(), 200);
}

/// 用于测试的简单 Mock 屏幕捕获器。
struct MockCapturer {
    frames: parking_lot::Mutex<Vec<CapturedFrame>>,
}

impl ScreenCapturer for MockCapturer {
    fn list_monitors(&self) -> HalResult<Vec<DisplayMonitorInfo>> {
        Ok(vec![])
    }
    fn capture_monitor(&self, _id: Option<u32>) -> HalResult<CapturedFrame> {
        unimplemented!()
    }
    fn capture_region(&self, _b: app_core::SelectionBox) -> HalResult<CapturedFrame> {
        let mut guard = self.frames.lock();
        if guard.is_empty() {
            // 没有更多帧时返回空画面
            Ok(CapturedFrame {
                width: 60,
                height: 100,
                scale_factor_milli: 1000,
                rgba_bytes: vec![0u8; 60 * 100 * 4],
            })
        } else {
            Ok(guard.remove(0))
        }
    }
}

#[test]
fn test_scroll_session_cancellation_releases_all() {
    let width = 60;
    let height = 100;
    let page = generate_synthetic_page(width, 300);

    let f0 = slice_frame(&page, width, 0, height);
    let f1 = slice_frame(&page, width, 30, height);

    let capturer = Arc::new(MockCapturer {
        frames: parking_lot::Mutex::new(vec![
            CapturedFrame {
                width: width as u32,
                height: height as u32,
                scale_factor_milli: 1000,
                rgba_bytes: f0,
            },
            CapturedFrame {
                width: width as u32,
                height: height as u32,
                scale_factor_milli: 1000,
                rgba_bytes: f1,
            },
        ]),
    });

    let selection = app_core::SelectionBox {
        x: 0,
        y: 0,
        width: width as u32,
        height: height as u32,
    };
    let session = ScrollCaptureSession::new(
        "test_cancel".to_string(),
        selection,
        selection,
        capturer,
        None,
        StitcherConfig::default(),
        None,
        0,
    );

    // 稍作等待以接收初始帧
    std::thread::sleep(std::time::Duration::from_millis(150));

    // 用户在进行中点击取消
    assert!(session.cancel());
    session.wait_finished(std::time::Duration::from_secs(1));

    let st = session.status();
    assert!(!st.is_active);
    assert_eq!(st.stop_reason, Some(StopReason::Cancelled));
    // 取消后结果必须被清理丢弃
    assert!(session.take_stitched_image().is_none());
}
