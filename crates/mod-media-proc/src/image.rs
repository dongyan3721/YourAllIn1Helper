use crate::{
    output::finalize_output,
    types::{ImageConvertOptions, ImageOutputFormat, MediaJob},
};
use app_core::{CoreError, CoreResult};
use image::{codecs::jpeg::JpegEncoder, codecs::webp::WebPEncoder, DynamicImage};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub fn process(
    job: &MediaJob,
    options: &ImageConvertOptions,
    partial: &Path,
    cancelled: &AtomicBool,
) -> CoreResult<()> {
    check_cancelled(cancelled)?;
    options.validate()?;
    let decoded = image::open(&job.input_path).map_err(|error| {
        CoreError::Config(format!(
            "cannot decode {}: {error}",
            job.input_path.display()
        ))
    })?;
    check_cancelled(cancelled)?;
    let resized = resize(decoded, options)?;
    check_cancelled(cancelled)?;
    let file = File::create(partial)?;
    let mut writer = BufWriter::new(file);
    match options.output_format {
        ImageOutputFormat::Jpeg => {
            let mut encoder =
                JpegEncoder::new_with_quality(&mut writer, options.quality.unwrap_or(85));
            let rgb = resized.to_rgb8();
            encoder
                .encode(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|error| CoreError::Config(format!("JPEG encoding failed: {error}")))?;
        }
        ImageOutputFormat::Webp => {
            let rgba = resized.to_rgba8();
            WebPEncoder::new_lossless(&mut writer)
                .encode(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|error| CoreError::Config(format!("WebP encoding failed: {error}")))?;
        }
        format => {
            let image_format = match format {
                ImageOutputFormat::Png => image::ImageFormat::Png,
                ImageOutputFormat::Bmp => image::ImageFormat::Bmp,
                ImageOutputFormat::Ico => image::ImageFormat::Ico,
                _ => unreachable!(),
            };
            if format == ImageOutputFormat::Ico {
                DynamicImage::ImageRgba8(resized.to_rgba8())
                    .write_to(&mut writer, image_format)
                    .map_err(|error| CoreError::Config(format!("ICO encoding failed: {error}")))?;
            } else {
                resized
                    .write_to(&mut writer, image_format)
                    .map_err(|error| {
                        CoreError::Config(format!("image encoding failed: {error}"))
                    })?;
            }
        }
    }
    writer.flush()?;
    check_cancelled(cancelled)?;
    let output = job
        .output_path
        .as_ref()
        .ok_or_else(|| CoreError::Config("missing image output path".into()))?;
    finalize_output(partial, output)
}

fn check_cancelled(cancelled: &AtomicBool) -> CoreResult<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(CoreError::Config("job cancelled".into()))
    } else {
        Ok(())
    }
}

fn resize(image: DynamicImage, options: &ImageConvertOptions) -> CoreResult<DynamicImage> {
    if options.width.is_none() && options.height.is_none() {
        return Ok(image);
    }
    let resized = if options.preserve_aspect {
        image.resize(
            options.width.unwrap_or(u32::MAX),
            options.height.unwrap_or(u32::MAX),
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        image.resize_exact(
            options.width.unwrap_or(image.width()),
            options.height.unwrap_or(image.height()),
            image::imageops::FilterType::Lanczos3,
        )
    };
    if resized.width() == 0 || resized.height() == 0 {
        return Err(CoreError::Config("resize produced an empty image".into()));
    }
    Ok(resized)
}
