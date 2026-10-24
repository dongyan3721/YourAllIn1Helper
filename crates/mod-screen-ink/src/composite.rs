//! 屏幕底图与透明墨迹图层的离线/在线 Alpha 合成引擎。

use anyhow::{anyhow, Context};
use app_core::{CoreError, CoreResult};
use image::{imageops, RgbaImage};
use std::io::Cursor;

/// 将透明墨迹 PNG 与屏幕底图（RGBA8）按像素 Alpha 混合，并返回最终合并的 PNG 字节与尺寸。
pub fn composite_screen_with_ink(
    bg_width: u32,
    bg_height: u32,
    bg_rgba: &[u8],
    ink_png_bytes: &[u8],
) -> CoreResult<(Vec<u8>, u32, u32)> {
    if bg_width == 0 || bg_height == 0 {
        return Err(CoreError::Internal(anyhow!(
            "Invalid background dimensions: {}x{}",
            bg_width,
            bg_height
        )));
    }
    let expected_len = (bg_width as usize) * (bg_height as usize) * 4;
    if bg_rgba.len() != expected_len {
        return Err(CoreError::Internal(anyhow!(
            "Background RGBA buffer size mismatch: expected {} bytes, got {}",
            expected_len,
            bg_rgba.len()
        )));
    }

    // 1. 载入屏幕底图
    let mut bg_image =
        RgbaImage::from_raw(bg_width, bg_height, bg_rgba.to_vec()).ok_or_else(|| {
            CoreError::Internal(anyhow!(
                "Failed to construct RGBA image from background buffer"
            ))
        })?;

    // 2. 解码前端传入的透明墨迹 PNG
    let ink_dynamic = image::load_from_memory(ink_png_bytes)
        .context("Failed to decode ink PNG image")
        .map_err(|e| CoreError::Internal(e))?;
    let ink_rgba = ink_dynamic.to_rgba8();

    // 3. 处理 HiDPI / 屏幕分辨率比例不一致问题（如 macOS 2x Retina 与逻辑 CSS 像素）
    let ink_resized = if ink_rgba.width() != bg_width || ink_rgba.height() != bg_height {
        imageops::resize(
            &ink_rgba,
            bg_width,
            bg_height,
            imageops::FilterType::Triangle,
        )
    } else {
        ink_rgba
    };

    // 4. Alpha 叠加合成：将透明墨迹图层叠加到底图之上
    imageops::overlay(&mut bg_image, &ink_resized, 0, 0);

    // 5. 编码为 PNG 格式输出
    let mut output_bytes = Vec::new();
    bg_image
        .write_to(&mut Cursor::new(&mut output_bytes), image::ImageFormat::Png)
        .context("Failed to encode composited image to PNG")
        .map_err(|e| CoreError::Internal(e))?;

    Ok((output_bytes, bg_width, bg_height))
}
