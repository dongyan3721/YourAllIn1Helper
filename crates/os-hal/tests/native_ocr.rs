//! macOS Vision OCR 真实识别测试：用自制 5×7 点阵字体渲染文字，不读取屏幕或用户文件。
#![cfg(target_os = "macos")]

use os_hal::{CapturedFrame, NativeOsHal, OcrBackendKind, OcrProvider};

/// 5×7 点阵字形（每行 5 位，高位在左）。
fn glyph(c: char) -> [u8; 7] {
    match c {
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        _ => [0; 7],
    }
}

/// 黑字白底渲染一行文字，每个点放大为 `scale`×`scale` 像素。
fn render(text: &str, scale: u32) -> CapturedFrame {
    let margin = scale * 4;
    let advance = 6 * scale;
    let width = margin * 2 + advance * text.chars().count() as u32;
    let height = margin * 2 + 7 * scale;
    let mut rgba = vec![255u8; (width * height * 4) as usize];
    for (index, c) in text.chars().enumerate() {
        let rows = glyph(c);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (0x10 >> col) == 0 {
                    continue;
                }
                let x0 = margin + index as u32 * advance + col * scale;
                let y0 = margin + row as u32 * scale;
                for y in y0..y0 + scale {
                    for x in x0..x0 + scale {
                        let i = ((y * width + x) * 4) as usize;
                        rgba[i..i + 3].copy_from_slice(&[0, 0, 0]);
                    }
                }
            }
        }
    }
    CapturedFrame {
        width,
        height,
        scale_factor_milli: 1000,
        rgba_bytes: rgba,
    }
}

#[test]
fn vision_recognizes_rendered_text_with_pixel_coordinates() {
    let frame = render("OCR TEST 2026", 8);
    let result = NativeOsHal::new()
        .recognize_rgba(&frame)
        .expect("Vision OCR should run offline without extra permission");
    assert_eq!(result.backend_used, OcrBackendKind::SystemNative);
    let normalized: String = result
        .full_text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    assert!(
        normalized.contains("2026") && normalized.contains("TEST"),
        "unexpected OCR text: {:?}",
        result.full_text
    );
    // 坐标为左上原点的帧像素，必须落在图像内。
    for line in &result.lines {
        assert!(line.x + line.width <= frame.width + 1, "{line:?}");
        assert!(line.y + line.height <= frame.height + 1, "{line:?}");
        assert!((0.0..=1.0).contains(&line.confidence));
    }
}

#[test]
fn vision_returns_empty_result_for_blank_image() {
    let frame = render("", 8);
    let result = NativeOsHal::new().recognize_rgba(&frame).unwrap();
    assert!(result.full_text.is_empty());
    assert!(result.lines.is_empty());
}
