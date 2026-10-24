use app_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageOutputFormat {
    Png,
    Jpeg,
    Webp,
    Bmp,
    Ico,
}

impl ImageOutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
            Self::Bmp => "bmp",
            Self::Ico => "ico",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
            Self::Bmp => "bmp",
            Self::Ico => "ico",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageConvertOptions {
    pub output_format: ImageOutputFormat,
    pub quality: Option<u8>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub preserve_aspect: bool,
}

impl Default for ImageConvertOptions {
    fn default() -> Self {
        Self {
            output_format: ImageOutputFormat::Webp,
            quality: None,
            width: None,
            height: None,
            preserve_aspect: true,
        }
    }
}

impl ImageConvertOptions {
    pub fn validate(&self) -> CoreResult<()> {
        if self
            .quality
            .is_some_and(|quality| !(1..=100).contains(&quality))
        {
            return Err(CoreError::Config(
                "quality must be between 1 and 100".into(),
            ));
        }
        if matches!(self.width, Some(0)) || matches!(self.height, Some(0)) {
            return Err(CoreError::Config(
                "image dimensions must be positive".into(),
            ));
        }
        if self.width.is_some_and(|v| v > 32_768) || self.height.is_some_and(|v| v > 32_768) {
            return Err(CoreError::Config(
                "image dimensions cannot exceed 32768 pixels".into(),
            ));
        }
        if !self.preserve_aspect && (self.width.is_none() || self.height.is_none()) {
            return Err(CoreError::Config(
                "width and height are both required when preserve_aspect is false".into(),
            ));
        }
        if self.quality.is_some() && self.output_format != ImageOutputFormat::Jpeg {
            return Err(CoreError::Config(
                "quality is currently supported only for JPEG; WebP uses lossless encoding".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvOperation {
    Convert,
    Trim,
    ExtractAudio,
    AdjustVolume,
}

impl AvOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Convert => "convert",
            Self::Trim => "trim",
            Self::ExtractAudio => "extract_audio",
            Self::AdjustVolume => "adjust_volume",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvJobRequest {
    pub input_path: PathBuf,
    pub operation: AvOperation,
    pub output_format: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub volume: Option<f64>,
    #[serde(default)]
    pub precise: bool,
}

impl AvJobRequest {
    pub fn validate(&self) -> CoreResult<()> {
        if !self.input_path.is_file() {
            return Err(CoreError::Config(format!(
                "input file does not exist: {}",
                self.input_path.display()
            )));
        }
        if self
            .start_seconds
            .is_some_and(|v| !v.is_finite() || v < 0.0)
            || self.end_seconds.is_some_and(|v| !v.is_finite() || v < 0.0)
        {
            return Err(CoreError::Config(
                "trim times must be finite and >= 0".into(),
            ));
        }
        if self.operation == AvOperation::Trim {
            let start = self
                .start_seconds
                .ok_or_else(|| CoreError::Config("start_seconds is required for trim".into()))?;
            let end = self
                .end_seconds
                .ok_or_else(|| CoreError::Config("end_seconds is required for trim".into()))?;
            if end <= start {
                return Err(CoreError::Config(
                    "end_seconds must be greater than start_seconds".into(),
                ));
            }
        } else if self.start_seconds.is_some() || self.end_seconds.is_some() {
            return Err(CoreError::Config(
                "trim times are only valid for the trim operation".into(),
            ));
        }
        if let Some(volume) = self.volume {
            if !volume.is_finite() || !(0.0..=10.0).contains(&volume) {
                return Err(CoreError::Config("volume must be between 0 and 10".into()));
            }
            if self.operation != AvOperation::AdjustVolume {
                return Err(CoreError::Config(
                    "volume is only valid for adjust_volume".into(),
                ));
            }
        } else if self.operation == AvOperation::AdjustVolume {
            return Err(CoreError::Config(
                "volume is required for adjust_volume".into(),
            ));
        }
        if let Some(format) = &self.output_format {
            let normalized = format.trim().trim_start_matches('.').to_ascii_lowercase();
            let allowed = match self.operation {
                AvOperation::ExtractAudio | AvOperation::AdjustVolume => {
                    ["mp3", "wav", "flac", "aac", "m4a", "ogg"].as_slice()
                }
                _ => [
                    "mp3", "wav", "flac", "aac", "m4a", "ogg", "mp4", "mkv", "mov", "webm",
                ]
                .as_slice(),
            };
            if !allowed.contains(&normalized.as_str()) {
                return Err(CoreError::Config(format!(
                    "unsupported output format '{format}' for {}",
                    self.operation.as_str()
                )));
            }
        } else if self.operation == AvOperation::Convert
            || self.operation == AvOperation::ExtractAudio
            || self.operation == AvOperation::AdjustVolume
        {
            return Err(CoreError::Config("output_format is required".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl MediaJobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaJob {
    pub id: String,
    pub operation: String,
    pub input_path: PathBuf,
    pub output_path: Option<PathBuf>,
    pub status: MediaJobStatus,
    pub progress: f64,
    pub speed: Option<String>,
    pub error: Option<String>,
    pub metadata: Option<Value>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl MediaJob {
    pub fn new(operation: impl Into<String>, input_path: PathBuf) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            operation: operation.into(),
            input_path,
            output_path: None,
            status: MediaJobStatus::Queued,
            progress: 0.0,
            speed: None,
            error: None,
            metadata: None,
            created_at: now,
            updated_at: now,
        }
    }
}
