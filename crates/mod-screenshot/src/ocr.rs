//! 截图 M2：选区文字识别（OCR）的领域类型与适配。
//!
//! 识别由注入的 `os_hal::OcrProvider` 完成（macOS Vision）；本模块只负责裁剪冻结帧原图
//! （不含标注层，避免把箭头/马赛克误识为文字）、校验尺寸与整理结果。
//! 宿主未注入时使用 [`UnavailableOcr`]，如实返回「不支持」，绝不返回示例文本。
use os_hal::{
    CapturedFrame, HalError, HalResult, OcrBackendKind, OcrLineResult, OcrProvider,
    OcrRecognitionResult,
};
use serde::{Deserialize, Serialize};

/// 单次识别允许的最大像素数（约 4000 万），超出时拒绝以免长时间占用。
pub const MAX_OCR_PIXELS: u64 = 40_000_000;
/// 复制识别文本的最大字符数。
pub const MAX_COPY_TEXT_CHARS: usize = 200_000;

/// 截图会话的触发意图：普通截图或「截图并识别文字」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureIntent {
    #[default]
    Capture,
    Ocr,
}

/// 识别结果；行坐标为相对选区左上角的帧像素。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrOutcome {
    pub backend: OcrBackendKind,
    pub text: String,
    pub lines: Vec<OcrLineResult>,
    pub elapsed_ms: u64,
    pub width: u32,
    pub height: u32,
}

impl OcrOutcome {
    pub fn from_result(result: OcrRecognitionResult, width: u32, height: u32) -> Self {
        Self {
            backend: result.backend_used,
            text: result.full_text,
            lines: result.lines,
            elapsed_ms: result.elapsed_ms,
            width,
            height,
        }
    }
}

/// 宿主未注入 OCR 能力时的实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableOcr;

impl OcrProvider for UnavailableOcr {
    fn recognize_rgba(&self, _frame: &CapturedFrame) -> HalResult<OcrRecognitionResult> {
        Err(HalError::Unsupported("native_ocr"))
    }

    fn release_idle_sessions(&self) -> HalResult<()> {
        Ok(())
    }
}
