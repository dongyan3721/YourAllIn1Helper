//! Windows 平台系统原生 OCR：`Windows.Media.Ocr`（WinRT 引擎，离线、零额外打包体积）。
//!
//! 职责与设计（与 macOS Vision 分支保持同等“能力诚实”原则）：
//! 1. 只使用系统已安装的 OCR 语言包；无语言包/语言不受支持时明确返回 `Unsupported`，
//!    绝不返回示例文本。
//! 2. 输入为 HAL 统一 RGBA8 冻结帧；CPU 侧先用 `image` 的标准 luminance 公式转 8bit 灰度，
//!    封装为无压缩灰度 BMP 内存流，经 `BitmapDecoder` 解码为 `SoftwareBitmap`（Gray8）后识别。
//! 3. 语言选择：按 `PREFERRED_LANGUAGES` 顺序调用 `OcrEngine::TryCreateFromLanguage`；
//!    全部缺失时回退 `TryCreateFromUserProfileLanguages`；仍失败则如实报告 `Unsupported`。
//! 4. 每次识别在专用工作线程初始化 WinRT MTA，避免阻塞 UI / STA 线程。
//! 5. 识别全程串行化（对齐 macOS VISION_LOCK：规避冷启动并发竞争并限制大图内存峰值）；
//!    WinRT 异步操作通过 `windows-future` 的 `join()` 完成后再取结果。
//! 6. Windows.Media.Ocr 不提供行级置信度，按 1.0 如实报告，不伪造中间值；
//!    行框由词级 `OcrWord::BoundingRect` 求并集得到。

use crate::types::{
    CapturedFrame, HalError, HalResult, OcrBackendKind, OcrLineResult, OcrRecognitionResult,
};

/// 优先识别的语言（BCP-47，与系统已安装 OCR 语言包求交集后按此顺序设置）。
const PREFERRED_LANGUAGES: &[&str] = &["zh-Hans-CN", "zh-Hant-TW", "en-US", "ja-JP", "ko-KR"];

/// 串行化 WinRT OCR 调用。
static WINOCR_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// 在 Windows.Media.Ocr 引擎上执行一次同步识别，返回按阅读顺序排列的真实结果。
pub fn recognize_with_winocr(frame: &CapturedFrame) -> HalResult<OcrRecognitionResult> {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("windows-native-ocr".into())
            .spawn_scoped(scope, || recognize_on_worker(frame))
            .map_err(|err| {
                HalError::NativeCallFailed(format!("启动 Windows OCR 工作线程失败：{err}"))
            })?;
        worker
            .join()
            .map_err(|_| HalError::NativeCallFailed("Windows OCR 工作线程意外终止".into()))?
    })
}

fn recognize_on_worker(frame: &CapturedFrame) -> HalResult<OcrRecognitionResult> {
    let _serial = WINOCR_LOCK.lock();
    let started = std::time::Instant::now();

    let _apartment = WinRtApartment::initialize()?;

    use windows::Foundation::Rect;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    let engine = create_engine()?;
    let max_dimension = OcrEngine::MaxImageDimension()
        .map_err(|err| HalError::NativeCallFailed(format!("查询 OCR 最大图像尺寸失败：{err}")))?;
    if frame.width > max_dimension || frame.height > max_dimension {
        return Err(HalError::NativeCallFailed(format!(
            "OCR 图像尺寸 {}x{} 超出系统上限 {max_dimension}，请缩小选区",
            frame.width, frame.height
        )));
    }
    let gray = to_gray8(frame)?;
    let bmp_bytes = encode_gray_bmp(frame.width, frame.height, &gray)?;

    let failed = |what: &str, err: windows::core::Error| {
        HalError::NativeCallFailed(format!("{what}失败：{err}"))
    };

    // 灰度 BMP → 内存流 → BitmapDecoder → SoftwareBitmap（Gray8，OCR 推荐输入）。
    let stream = InMemoryRandomAccessStream::new().map_err(|err| failed("创建内存流", err))?;
    let mut lines: Vec<OcrLineResult> = Vec::new();
    let output = stream
        .GetOutputStreamAt(0)
        .map_err(|err| failed("获取内存流输出端", err))?;
    let writer =
        DataWriter::CreateDataWriter(&output).map_err(|err| failed("创建 DataWriter", err))?;
    writer
        .WriteBytes(&bmp_bytes)
        .map_err(|err| failed("写入内存流", err))?;
    writer
        .StoreAsync()
        .map_err(|err| failed("写入内存流", err))?
        .join()
        .map_err(|err| failed("写入内存流", err))?;
    if !writer
        .FlushAsync()
        .map_err(|err| failed("冲刷内存流", err))?
        .join()
        .map_err(|err| failed("冲刷内存流", err))?
    {
        return Err(HalError::NativeCallFailed("冲刷 OCR 输入内存流失败".into()));
    }
    stream.Seek(0).map_err(|err| failed("回卷内存流", err))?;

    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|err| failed("解码 BMP", err))?
        .join()
        .map_err(|err| failed("解码 BMP", err))?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|err| failed("获取位图", err))?
        .join()
        .map_err(|err| failed("获取位图", err))?;

    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|err| failed("启动 OCR", err))?
        .join()
        .map_err(|err| failed("OCR 识别", err))?;

    let ocr_lines = result.Lines().map_err(|err| failed("读取 OCR 行", err))?;
    let count = ocr_lines
        .Size()
        .map_err(|err| failed("读取 OCR 行数", err))?;
    for index in 0..count {
        let line = ocr_lines
            .GetAt(index)
            .map_err(|err| failed("读取 OCR 行", err))?;
        let text = line
            .Text()
            .map_err(|err| failed("读取 OCR 文本", err))?
            .to_string();
        if text.trim().is_empty() {
            continue;
        }
        // OcrLine 不直接提供整行包围盒：用词级 BoundingRect 求并集（像素坐标）。
        let bounds = union_of_word_rects(&line).unwrap_or(Rect {
            X: 0.0,
            Y: 0.0,
            Width: frame.width as f32,
            Height: frame.height as f32,
        });
        lines.push(OcrLineResult {
            text,
            // Windows.Media.Ocr 不提供行级置信度；如实按 1.0 报告，不伪造中间值。
            confidence: 1.0,
            x: bounds.X.max(0.0).round() as u32,
            y: bounds.Y.max(0.0).round() as u32,
            width: bounds.Width.max(0.0).round() as u32,
            height: bounds.Height.max(0.0).round() as u32,
        });
    }

    // 按阅读顺序（自上而下、自左而右）排列，行高一半以内视为同一行（与 macOS 分支一致）。
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

/// 用词级 `OcrWord::BoundingRect` 并集近似整行包围盒；无词时返回 None 由调用方兜底。
fn union_of_word_rects(line: &windows::Media::Ocr::OcrLine) -> Option<windows::Foundation::Rect> {
    let words = line.Words().ok()?;
    let count = words.Size().ok()?;
    if count == 0 {
        return None;
    }
    let mut left = f32::MAX;
    let mut top = f32::MAX;
    let mut right = f32::MIN;
    let mut bottom = f32::MIN;
    for index in 0..count {
        let word = words.GetAt(index).ok()?;
        let rect = word.BoundingRect().ok()?;
        left = left.min(rect.X);
        top = top.min(rect.Y);
        right = right.max(rect.X + rect.Width);
        bottom = bottom.max(rect.Y + rect.Height);
    }
    if right <= left || bottom <= top {
        return None;
    }
    Some(windows::Foundation::Rect {
        X: left,
        Y: top,
        Width: right - left,
        Height: bottom - top,
    })
}

/// 构建 OCR 引擎：优先语言列表逐个尝试，全部缺失时回退用户配置文件语言。
fn create_engine() -> HalResult<windows::Media::Ocr::OcrEngine> {
    use windows::Globalization::Language;
    use windows::Media::Ocr::OcrEngine;

    for tag in PREFERRED_LANGUAGES {
        let htag = windows::core::HSTRING::from(*tag);
        if let Ok(language) = Language::CreateLanguage(&htag) {
            // Err 表示该语言不受系统 OCR 支持（TryCreate 返回空被包装为错误），继续下一项。
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
                return Ok(engine);
            }
        }
    }
    OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|_| HalError::Unsupported("windows_ocr_language_pack_missing"))
}

/// RGBA8 → 8bit 灰度（image crate 的标准 luminance 转换，不读越界）。
fn to_gray8(frame: &CapturedFrame) -> HalResult<Vec<u8>> {
    use image::{DynamicImage, RgbaImage};
    let rgba = RgbaImage::from_raw(frame.width, frame.height, frame.rgba_bytes.clone())
        .ok_or_else(|| HalError::NativeCallFailed("OCR RGBA 图像缓冲区尺寸无效".into()))?;
    Ok(DynamicImage::ImageRgba8(rgba).to_luma8().into_raw())
}

/// 将灰度栅格封装为无压缩 8bit 灰度 BMP（自顶向下负高度），供 BitmapDecoder 解码。
fn encode_gray_bmp(width: u32, height: u32, gray: &[u8]) -> HalResult<Vec<u8>> {
    let width_usize = usize::try_from(width)
        .map_err(|_| HalError::NativeCallFailed("OCR 图像宽度超出平台限制".into()))?;
    let height_usize = usize::try_from(height)
        .map_err(|_| HalError::NativeCallFailed("OCR 图像高度超出平台限制".into()))?;
    let row_stride = width_usize
        .checked_add(3)
        .map(|stride| stride & !3usize)
        .ok_or_else(|| HalError::NativeCallFailed("OCR BMP 行尺寸溢出".into()))?;
    let pixel_array_size = row_stride
        .checked_mul(height_usize)
        .filter(|size| *size <= u32::MAX as usize)
        .ok_or_else(|| HalError::NativeCallFailed("OCR BMP 图像数据过大".into()))?;
    let expected_gray_size = width_usize
        .checked_mul(height_usize)
        .ok_or_else(|| HalError::NativeCallFailed("OCR 灰度图尺寸溢出".into()))?;
    if width == 0 || height == 0 || gray.len() != expected_gray_size {
        return Err(HalError::NativeCallFailed(
            "OCR 灰度图缓冲区尺寸无效".into(),
        ));
    }
    let file_size = 14usize
        .checked_add(40 + 256 * 4)
        .and_then(|header_size| header_size.checked_add(pixel_array_size))
        .filter(|size| *size <= u32::MAX as usize)
        .ok_or_else(|| HalError::NativeCallFailed("OCR BMP 文件过大".into()))?;

    let mut out = Vec::new();
    out.reserve(file_size);

    fn put_u16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    fn put_u32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    fn put_i32(out: &mut Vec<u8>, v: i32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    out.extend_from_slice(b"BM");
    put_u32(&mut out, file_size as u32);
    put_u32(&mut out, 0);
    put_u32(&mut out, (14 + 40 + 256 * 4) as u32);
    // BITMAPINFOHEADER
    put_u32(&mut out, 40);
    put_i32(
        &mut out,
        i32::try_from(width)
            .map_err(|_| HalError::NativeCallFailed("OCR 图像宽度超出 BMP 范围".into()))?,
    );
    // 负高度 = 自顶向下的行序，与输入灰度数据一致。
    put_i32(
        &mut out,
        i32::try_from(height)
            .ok()
            .and_then(i32::checked_neg)
            .ok_or_else(|| HalError::NativeCallFailed("OCR 图像高度超出 BMP 范围".into()))?,
    );
    put_u16(&mut out, 1);
    put_u16(&mut out, 8);
    put_u32(&mut out, 0);
    put_u32(&mut out, pixel_array_size as u32);
    put_i32(&mut out, 2835);
    put_i32(&mut out, 2835);
    put_u32(&mut out, 256);
    put_u32(&mut out, 0);
    // 灰度调色板：索引 0..=255 线性映射到同名灰阶。
    for level in 0u16..=255u16 {
        let byte = level as u8;
        out.extend_from_slice(&[byte, byte, byte, 0]);
    }
    // 像素数据：8bit 索引即灰度值，按行补齐 4 字节对齐。
    for row in gray.chunks_exact(width_usize) {
        out.extend_from_slice(row);
        out.resize(out.len() + (row_stride - width_usize), 0);
    }
    Ok(out)
}

struct WinRtApartment;

impl WinRtApartment {
    fn initialize() -> HalResult<Self> {
        use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED).map_err(|err| {
                HalError::NativeCallFailed(format!("WinRT COM 初始化失败：{err}"))
            })?;
        }
        Ok(Self)
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        use windows::Win32::System::WinRT::RoUninitialize;
        unsafe { RoUninitialize() };
    }
}
