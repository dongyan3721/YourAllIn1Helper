//! 流式 GIF 编码器。
//!
//! 基于 MIT/Apache 许可的 `gif` 与 `color_quant`，支持缩放、
//! NeuQuant 256 色自适应量化以及流式向磁盘写入帧，内存占用低且稳定。

use super::options::GifFramerate;
use anyhow::{anyhow, Result};
use color_quant::NeuQuant;
use gif::{Encoder, Frame, Repeat};
use image::{imageops::FilterType, RgbaImage};
use std::io::Write;

/// 流式 GIF 编码器。
pub struct GifStreamEncoder<W: Write> {
    encoder: Option<Encoder<W>>,
    target_width: u32,
    target_height: u32,
    framerate: GifFramerate,
    sample_factor: i32,
    encoded_frames: usize,
}

impl<W: Write> GifStreamEncoder<W> {
    pub fn new(
        writer: W,
        target_width: u32,
        target_height: u32,
        framerate: GifFramerate,
        sample_factor: i32,
    ) -> Result<Self> {
        let mut encoder = Encoder::new(writer, target_width as u16, target_height as u16, &[])
            .map_err(|e| anyhow!("初始化 GIF 编码器失败：{}", e))?;
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| anyhow!("设置 GIF 循环模式失败：{}", e))?;

        Ok(Self {
            encoder: Some(encoder),
            target_width,
            target_height,
            framerate,
            sample_factor: sample_factor.clamp(1, 30),
            encoded_frames: 0,
        })
    }

    pub fn encoded_frames(&self) -> usize {
        self.encoded_frames
    }

    /// 向 GIF 编码器追加一帧 RGBA 画面。
    pub fn add_frame(&mut self, rgba_bytes: &[u8], src_w: u32, src_h: u32) -> Result<()> {
        let encoder = self
            .encoder
            .as_mut()
            .ok_or_else(|| anyhow!("GIF 编码器已关闭"))?;

        // 1. 若原始尺寸与目标尺寸不一致，使用 image 进行高质量平滑缩放
        let target_rgba = if src_w == self.target_width && src_h == self.target_height {
            rgba_bytes.to_vec()
        } else {
            let img = RgbaImage::from_raw(src_w, src_h, rgba_bytes.to_vec())
                .ok_or_else(|| anyhow!("无法解析帧像素缓冲区为图像"))?;
            let resized = image::imageops::resize(
                &img,
                self.target_width,
                self.target_height,
                FilterType::Triangle,
            );
            resized.into_raw()
        };

        // 2. NeuQuant 256 色自适应量化
        let nq = NeuQuant::new(self.sample_factor, 256, &target_rgba);
        let palette = nq.color_map_rgb();

        let pixel_count = (self.target_width * self.target_height) as usize;
        let mut indexed_pixels = Vec::with_capacity(pixel_count);

        for chunk in target_rgba.chunks_exact(4) {
            let color = [chunk[0], chunk[1], chunk[2], chunk[3]];
            indexed_pixels.push(nq.index_of(&color) as u8);
        }

        // 3. 构建 GIF Frame 并写入流
        let mut frame = Frame::default();
        frame.width = self.target_width as u16;
        frame.height = self.target_height as u16;
        frame.delay = self.framerate.delay_centisecs();
        frame.palette = Some(palette);
        frame.buffer = std::borrow::Cow::Owned(indexed_pixels);

        encoder
            .write_frame(&frame)
            .map_err(|e| anyhow!("写入 GIF 帧数据失败：{}", e))?;

        self.encoded_frames += 1;
        Ok(())
    }

    /// 结束编码并刷新缓冲区，关闭写入。
    pub fn finish(mut self) -> Result<usize> {
        if let Some(encoder) = self.encoder.take() {
            drop(encoder);
        }
        Ok(self.encoded_frames)
    }
}
