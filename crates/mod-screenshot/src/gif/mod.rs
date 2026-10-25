//! GIF 录制模块：分辨率预设、流式 NeuQuant 量化编码与后台录制生命周期。

pub mod encoder;
pub mod options;
pub mod recorder;

pub use encoder::GifStreamEncoder;
pub use options::{
    calculate_target_dimensions, GifFramerate, GifRecordingConfig, GifResolutionPreset,
};
pub use recorder::{
    GifEventSink, GifRecordingSession, GifRecordingStatsPayload, GifRecordingStatus,
};

#[cfg(test)]
#[path = "../../tests/unit/gif.rs"]
mod tests;
