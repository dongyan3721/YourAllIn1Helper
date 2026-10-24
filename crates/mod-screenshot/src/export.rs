//! 选区成图：从冻结帧裁剪、叠加前端标注层、编码 PNG 与防覆盖落盘。
use crate::error::internal;
use crate::geometry::crop_rgba;
use app_core::{CoreResult, SelectionBox};
use image::{imageops, RgbaImage};
use os_hal::CapturedFrame;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

/// 标注层 PNG 的最大字节数，防止异常请求占用大量内存。
pub const MAX_OVERLAY_PNG_BYTES: usize = 128 * 1024 * 1024;
/// 同名文件追加序号的最大尝试次数。
const MAX_NAME_ATTEMPTS: u32 = 1000;

/// 前端选择的导出动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportAction {
    Copy,
    Save,
    Pin,
}

/// 裁剪冻结帧并叠加标注层；标注层必须与选区像素尺寸完全一致。
///
/// 标注层由前端按帧像素渲染（含马赛克块），后端只做 Alpha 叠加，原始帧不回传前端。
pub fn render_selection(
    frame: &CapturedFrame,
    selection: &SelectionBox,
    overlay_png: Option<&[u8]>,
) -> CoreResult<RgbaImage> {
    let cropped = crop_rgba(&frame.rgba_bytes, frame.width, frame.height, selection)?;
    let mut base = RgbaImage::from_raw(selection.width, selection.height, cropped)
        .ok_or_else(|| internal("无法由裁剪像素构建图像"))?;
    if let Some(png) = overlay_png.filter(|bytes| !bytes.is_empty()) {
        if png.len() > MAX_OVERLAY_PNG_BYTES {
            return Err(internal("标注层数据过大"));
        }
        let layer = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .map_err(|e| internal(format!("标注层 PNG 解码失败：{}", e)))?
            .to_rgba8();
        if layer.dimensions() != (selection.width, selection.height) {
            return Err(internal(format!(
                "标注层尺寸 {}x{} 与选区 {}x{} 不一致",
                layer.width(),
                layer.height(),
                selection.width,
                selection.height
            )));
        }
        imageops::overlay(&mut base, &layer, 0, 0);
    }
    Ok(base)
}

/// 编码为 PNG 字节。
pub fn encode_png(image: &RgbaImage) -> CoreResult<Vec<u8>> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| internal(format!("PNG 编码失败：{}", e)))?;
    Ok(bytes)
}

/// 生成带时间戳的文件名主干。
pub fn timestamped_stem(prefix: &str, now: chrono::DateTime<chrono::Local>) -> String {
    format!("{}_{}", prefix, now.format("%Y%m%d_%H%M%S"))
}

/// 以 `create_new` 独占创建文件写入；同名时追加 `_1`、`_2`…，绝不覆盖已有文件。
pub fn write_exclusive(directory: &Path, stem: &str, bytes: &[u8]) -> CoreResult<PathBuf> {
    fs::create_dir_all(directory)
        .map_err(|e| internal(format!("无法创建保存目录 {}：{}", directory.display(), e)))?;
    for attempt in 0..MAX_NAME_ATTEMPTS {
        let name = if attempt == 0 {
            format!("{}.png", stem)
        } else {
            format!("{}_{}.png", stem, attempt)
        };
        let path = directory.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                let mut writer = std::io::BufWriter::new(file);
                let written = writer.write_all(bytes).and_then(|_| writer.flush());
                if let Err(error) = written {
                    drop(writer);
                    // 写入失败时删除本次新建的半成品，不影响任何已有文件。
                    let _ = fs::remove_file(&path);
                    return Err(internal(format!(
                        "写入截图文件 {} 失败：{}",
                        path.display(),
                        error
                    )));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(internal(format!(
                    "创建截图文件 {} 失败：{}",
                    path.display(),
                    error
                )))
            }
        }
    }
    Err(internal(format!(
        "{} 次尝试后仍无法分配不重名的文件名",
        MAX_NAME_ATTEMPTS
    )))
}

#[cfg(test)]
#[path = "../tests/unit/export.rs"]
mod tests;
