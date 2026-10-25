//! 系统原生 OCR：macOS 使用 Vision `VNRecognizeTextRequest`（Accurate 级别，离线、无需额外权限）。
//!
//! Windows.Media.Ocr 与自定义 ONNX 模型尚未接入，其他平台明确返回 `Unsupported`，不返回示例文本。
use super::NativeOsHal;
use crate::{CapturedFrame, HalError, HalResult, OcrProvider, OcrRecognitionResult};

/// 优先识别的语言（与系统支持列表求交集后按此顺序设置）。
#[cfg(target_os = "macos")]
const PREFERRED_LANGUAGES: &[&str] = &["zh-Hans", "zh-Hant", "en-US", "ja-JP", "ko-KR"];

impl OcrProvider for NativeOsHal {
    fn recognize_rgba(&self, frame: &CapturedFrame) -> HalResult<OcrRecognitionResult> {
        validate_frame(frame)?;
        #[cfg(target_os = "macos")]
        {
            // Vision 冷启动加载模型期间并发请求会偶发 CRImageReaderError；串行化识别，
            // 同时限制大图并发带来的内存峰值。
            static VISION_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
            let _serial = VISION_LOCK.lock();
            return objc2::rc::autoreleasepool(|_| recognize_with_vision(frame));
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(HalError::Unsupported("native_ocr"))
        }
    }

    /// 系统 OCR 不持有常驻会话；自定义 ONNX 会话接入后在此释放。
    fn release_idle_sessions(&self) -> HalResult<()> {
        Ok(())
    }
}

fn validate_frame(frame: &CapturedFrame) -> HalResult<()> {
    let expected = frame.width as usize * frame.height as usize * 4;
    if frame.width == 0 || frame.height == 0 || frame.rgba_bytes.len() != expected {
        return Err(HalError::NativeCallFailed(format!(
            "OCR 输入尺寸无效：{}x{}，{} 字节",
            frame.width,
            frame.height,
            frame.rgba_bytes.len()
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn recognize_with_vision(frame: &CapturedFrame) -> HalResult<OcrRecognitionResult> {
    use crate::{OcrBackendKind, OcrLineResult};
    use objc2::{rc::Retained, AnyThread};
    use objc2_foundation::{NSArray, NSDictionary, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };

    let started = std::time::Instant::now();
    let image = rgba_to_cgimage(frame)?;

    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);
    // SAFETY: 只读查询系统支持的语言列表，错误时保持系统默认语言。
    if let Ok(supported) = unsafe { request.supportedRecognitionLanguagesAndReturnError() } {
        let supported: Vec<String> = supported.iter().map(|s| s.to_string()).collect();
        let chosen: Vec<Retained<NSString>> = PREFERRED_LANGUAGES
            .iter()
            .filter(|lang| supported.iter().any(|s| s == *lang))
            .map(|lang| NSString::from_str(lang))
            .collect();
        if !chosen.is_empty() {
            request.setRecognitionLanguages(&NSArray::from_retained_slice(&chosen));
        }
    }

    let options = NSDictionary::new();
    // SAFETY: CGImage 在本函数内存活；options 为空字典，类型与声明一致。
    let handler = unsafe {
        VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            &image,
            &options,
        )
    };
    // VNRecognizeTextRequest → VNImageBasedRequest → VNRequest。
    let as_request: Retained<VNRequest> =
        Retained::into_super(Retained::into_super(request.clone()));
    handler
        .performRequests_error(&NSArray::from_retained_slice(&[as_request]))
        .map_err(|error| {
            HalError::NativeCallFailed(format!(
                "Vision 文字识别失败：{}",
                error.localizedDescription()
            ))
        })?;

    let (w, h) = (frame.width as f64, frame.height as f64);
    let mut lines = Vec::new();
    for observation in request.results().iter().flat_map(|arr| arr.iter()) {
        let Some(best) = observation.topCandidates(1).iter().next() else {
            continue;
        };
        let text = best.string().to_string();
        if text.trim().is_empty() {
            continue;
        }
        // SAFETY: boundingBox 为 Vision 归一化坐标（原点左下），只读取数值。
        let bb = unsafe { observation.boundingBox() };
        let x = (bb.origin.x * w).clamp(0.0, w);
        let y = ((1.0 - bb.origin.y - bb.size.height) * h).clamp(0.0, h);
        lines.push(OcrLineResult {
            text,
            confidence: best.confidence(),
            x: x.round() as u32,
            y: y.round() as u32,
            width: (bb.size.width * w).round().max(0.0) as u32,
            height: (bb.size.height * h).round().max(0.0) as u32,
        });
    }
    // 按阅读顺序（自上而下、自左而右）排列，行高一半以内视为同一行。
    lines.sort_by(|a, b| {
        let same_row = (a.y as i64 - b.y as i64).abs() < (a.height.max(b.height) as i64 / 2).max(1);
        if same_row {
            a.x.cmp(&b.x)
        } else {
            a.y.cmp(&b.y)
        }
    });
    let full_text = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(OcrRecognitionResult {
        backend_used: OcrBackendKind::SystemNative,
        full_text,
        lines,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// RGBA8（直通 Alpha）→ CGImage；截图帧不透明，按 NoneSkipLast 忽略 Alpha。
#[cfg(target_os = "macos")]
fn rgba_to_cgimage(
    frame: &CapturedFrame,
) -> HalResult<objc2_core_foundation::CFRetained<objc2_core_graphics::CGImage>> {
    use objc2_core_foundation::CFData;
    use objc2_core_graphics::{
        CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
        CGImageAlphaInfo,
    };

    // SAFETY: 指针与长度来自同一个存活的 Vec；CFDataCreate 会复制字节。
    let data = unsafe {
        CFData::new(
            None,
            frame.rgba_bytes.as_ptr(),
            frame.rgba_bytes.len() as isize,
        )
    }
    .ok_or_else(|| HalError::NativeCallFailed("创建 CFData 失败".into()))?;
    let provider = CGDataProvider::with_cf_data(Some(&data))
        .ok_or_else(|| HalError::NativeCallFailed("创建 CGDataProvider 失败".into()))?;
    let space = CGColorSpace::new_device_rgb()
        .ok_or_else(|| HalError::NativeCallFailed("创建 RGB 色彩空间失败".into()))?;
    // SAFETY: 尺寸、步长与缓冲区长度已在 validate_frame 中校验；decode 传空指针。
    unsafe {
        CGImage::new(
            frame.width as usize,
            frame.height as usize,
            8,
            32,
            frame.width as usize * 4,
            Some(&space),
            CGBitmapInfo(CGImageAlphaInfo::NoneSkipLast.0),
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .ok_or_else(|| HalError::NativeCallFailed("创建 CGImage 失败".into()))
}
