//! 滚动长截图模块：NCC 匹配、特征初筛、画布拼接与后台采集会话。

pub mod ncc;
pub mod session;
pub mod stitcher;

pub use ncc::{DisplacementMatch, NccConfig};
pub use session::{
    ScrollCaptureSession, ScrollEventSink, ScrollFrameStitchedPayload, ScrollStatusBrief,
};
pub use stitcher::{ScrollStitcher, StitchStepResult, StitchedImage, StitcherConfig, StopReason};

#[cfg(test)]
#[path = "../../tests/unit/scroll.rs"]
mod tests;
